//! Wasm plugin host for Panda (Phase 2).
//!
//! Current ABI v0:
//! - guest export `panda_abi_version() -> i32` (must equal [`PANDA_WASM_ABI_VERSION`])
//! - optional guest export `panda_on_request() -> i32`
//! - optional guest export `panda_on_request_body(ptr: i32, len: i32) -> i32`
//! - host import `panda_set_header(name_ptr, name_len, value_ptr, value_len)`
//! - host import `panda_set_body(ptr, len)` for request body replacement
//!
//! A plugin may call `panda_set_header` during `panda_on_request` to append request
//! headers before upstream forwarding. If request body hooks are enabled by the host,
//! `panda_on_request_body` can replace the buffered body through `panda_set_body`.
//!
//! Return code semantics for `panda_on_request*`:
//! - `0`: allow
//! - `1`: reject policy denied
//! - `2`: reject malformed request
//! - any other non-zero: reject plugin-specific

use std::path::Path;
use std::sync::Mutex;

use http::header::{HeaderMap, HeaderName, HeaderValue};
use wasmtime::{Caller, Engine, Instance, Linker, Memory, Module, Store};

/// ABI version negotiated between `panda-proxy` and guest modules.
pub const PANDA_WASM_ABI_VERSION: u32 = 0;
pub const RC_ALLOW: i32 = 0;
pub const RC_REJECT_POLICY_DENIED: i32 = 1;
pub const RC_REJECT_MALFORMED_REQUEST: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyCode {
    Denied,
    MalformedRequest,
    PluginSpecific(i32),
}

impl PolicyCode {
    fn from_rc(rc: i32) -> Option<Self> {
        if rc == RC_ALLOW {
            return None;
        }
        Some(match rc {
            RC_REJECT_POLICY_DENIED => Self::Denied,
            RC_REJECT_MALFORMED_REQUEST => Self::MalformedRequest,
            n => Self::PluginSpecific(n),
        })
    }
}

#[derive(Debug)]
pub enum HookFailure {
    PolicyReject { plugin: String, code: PolicyCode },
    Runtime(anyhow::Error),
}

impl std::fmt::Display for HookFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PolicyReject { plugin, code } => {
                write!(f, "plugin {plugin} rejected request code={code:?}")
            }
            Self::Runtime(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for HookFailure {}

#[derive(Default)]
struct HostState {
    pending_headers: Vec<(String, String)>,
    pending_body: Option<Vec<u8>>,
}

struct PluginInner {
    store: Store<HostState>,
    instance: Instance,
}

enum HookCallError {
    Policy(PolicyCode),
    Runtime(anyhow::Error),
}

/// One guest module loaded from disk.
pub struct LoadedPlugin {
    pub name: String,
    inner: Mutex<PluginInner>,
}

impl LoadedPlugin {
    /// If the module exports `add(i32, i32) -> i32`, call `add(1, 2)` and log; otherwise no-op.
    pub fn invoke_add_smoke(&self) -> anyhow::Result<()> {
        let mut g = self.inner.lock().map_err(|e| anyhow::anyhow!("plugin mutex: {e}"))?;
        let add = {
            let inner = &mut *g;
            match inner
                .instance
                .get_typed_func::<(i32, i32), i32>(&mut inner.store, "add")
            {
                Ok(f) => f,
                Err(_) => return Ok(()),
            }
        };
        let r = add.call(&mut g.store, (1, 2))?;
        eprintln!("panda wasm plugin {}: add(1,2) = {}", self.name, r);
        Ok(())
    }

    fn apply_request_headers(&self, headers: &mut HeaderMap) -> Result<usize, HookCallError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|e| HookCallError::Runtime(anyhow::anyhow!("plugin mutex: {e}")))?;
        g.store.data_mut().pending_headers.clear();

        let on_request = {
            let inner = &mut *g;
            match inner
                .instance
                .get_typed_func::<(), i32>(&mut inner.store, "panda_on_request")
            {
                Ok(f) => f,
                Err(_) => return Ok(0),
            }
        };
        let rc = on_request
            .call(&mut g.store, ())
            .map_err(|e| HookCallError::Runtime(anyhow::anyhow!("panda_on_request call: {e}")))?;
        if let Some(code) = PolicyCode::from_rc(rc) {
            return Err(HookCallError::Policy(code));
        }

        let mut applied = 0usize;
        let pending = std::mem::take(&mut g.store.data_mut().pending_headers);
        for (name_raw, value_raw) in pending {
            let name = HeaderName::from_bytes(name_raw.as_bytes()).map_err(|_| {
                HookCallError::Runtime(anyhow::anyhow!(
                    "invalid header name from plugin: {name_raw:?}"
                ))
            })?;
            let value = HeaderValue::from_str(&value_raw).map_err(|_| {
                HookCallError::Runtime(anyhow::anyhow!("invalid header value from plugin"))
            })?;
            headers.append(name, value);
            applied += 1;
        }
        Ok(applied)
    }

    fn apply_request_body(&self, request_body: &[u8]) -> Result<Option<Vec<u8>>, HookCallError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|e| HookCallError::Runtime(anyhow::anyhow!("plugin mutex: {e}")))?;
        g.store.data_mut().pending_body = None;

        let on_request_body = {
            let inner = &mut *g;
            match inner
                .instance
                .get_typed_func::<(i32, i32), i32>(&mut inner.store, "panda_on_request_body")
            {
                Ok(f) => f,
                Err(_) => return Ok(None),
            }
        };

        let memory = {
            let inner = &mut *g;
            memory_from_store(inner).map_err(HookCallError::Runtime)?
        };
        write_guest_bytes(&mut g.store, &memory, 0, request_body).map_err(HookCallError::Runtime)?;

        let rc = on_request_body.call(&mut g.store, (0, request_body.len() as i32)).map_err(|e| {
            HookCallError::Runtime(anyhow::anyhow!("panda_on_request_body call: {e}"))
        })?;
        if let Some(code) = PolicyCode::from_rc(rc) {
            return Err(HookCallError::Policy(code));
        }
        Ok(g.store.data_mut().pending_body.take())
    }
}

/// All plugins for one process (shared across connections).
pub struct PluginRuntime {
    #[allow(dead_code)]
    engine: Engine,
    plugins: Vec<LoadedPlugin>,
}

impl PluginRuntime {
    /// Load `*.wasm` from `dir` if `Some`, else return `None`. Empty directory is allowed (warn only).
    pub fn load_optional(dir: Option<&Path>) -> anyhow::Result<Option<Self>> {
        let Some(dir) = dir else {
            return Ok(None);
        };
        if !dir.is_dir() {
            anyhow::bail!("plugins directory is not a directory: {}", dir.display());
        }
        let engine = Engine::default();
        let plugins = Self::load_from_directory(&engine, dir)?;
        if plugins.is_empty() {
            eprintln!("panda: no .wasm files in {}", dir.display());
        }
        Ok(Some(Self { engine, plugins }))
    }

    fn load_from_directory(engine: &Engine, dir: &Path) -> anyhow::Result<Vec<LoadedPlugin>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("wasm") {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let module = Module::from_file(engine, &path)
                .map_err(|err| anyhow::anyhow!("{}: {err}", path.display()))?;
            let mut linker: Linker<HostState> = Linker::new(engine);
            linker.func_wrap(
                "panda_host",
                "panda_set_header",
                |mut caller: Caller<'_, HostState>,
                 name_ptr: i32,
                 name_len: i32,
                 value_ptr: i32,
                 value_len: i32| {
                    let Ok(memory) = memory_from_caller(&mut caller) else {
                        return;
                    };
                    let Ok(name) = read_utf8(&caller, &memory, name_ptr, name_len) else {
                        return;
                    };
                    let Ok(value) = read_utf8(&caller, &memory, value_ptr, value_len) else {
                        return;
                    };
                    caller.data_mut().pending_headers.push((name, value));
                },
            )?;
            linker.func_wrap(
                "panda_host",
                "panda_set_body",
                |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
                    let Ok(memory) = memory_from_caller(&mut caller) else {
                        return;
                    };
                    let Ok(body) = read_bytes(&caller, &memory, ptr, len) else {
                        return;
                    };
                    caller.data_mut().pending_body = Some(body);
                },
            )?;

            let mut store = Store::new(engine, HostState::default());
            let instance = linker
                .instantiate(&mut store, &module)
                .map_err(|err| anyhow::anyhow!("{} instantiate: {err}", path.display()))?;
            validate_plugin_abi(&mut store, &instance, &path)?;
            out.push(LoadedPlugin {
                name,
                inner: Mutex::new(PluginInner { store, instance }),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Call `add` smoke on each plugin that exports it (logs errors, does not fail the gateway).
    pub fn smoke_test(&self) {
        for p in &self.plugins {
            if let Err(e) = p.invoke_add_smoke() {
                eprintln!("panda wasm plugin {} smoke: {e:#}", p.name);
            }
        }
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    pub fn apply_request_plugins_strict(&self, headers: &mut HeaderMap) -> Result<usize, HookFailure> {
        let mut total = 0usize;
        for p in &self.plugins {
            let n = p.apply_request_headers(headers).map_err(|e| {
                match e {
                    HookCallError::Policy(code) => HookFailure::PolicyReject {
                        plugin: p.name.clone(),
                        code,
                    },
                    HookCallError::Runtime(e) => HookFailure::Runtime(anyhow::anyhow!(
                        "plugin {} request hook failed: {e:#}",
                        p.name
                    )),
                }
            })?;
            total += n;
        }
        Ok(total)
    }

    pub fn apply_request_body_plugins_strict(
        &self,
        request_body: &[u8],
        max_output_bytes: usize,
    ) -> Result<Option<Vec<u8>>, HookFailure> {
        let mut current: Option<Vec<u8>> = None;
        for p in &self.plugins {
            let input = current.as_deref().unwrap_or(request_body);
            match p.apply_request_body(input) {
                Ok(Some(next)) => {
                    if next.len() > max_output_bytes {
                        return Err(HookFailure::Runtime(anyhow::anyhow!(
                            "plugin {} body hook output too large: {} > {}",
                            p.name,
                            next.len(),
                            max_output_bytes
                        )));
                    }
                    current = Some(next);
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(match e {
                        HookCallError::Policy(code) => HookFailure::PolicyReject {
                            plugin: p.name.clone(),
                            code,
                        },
                        HookCallError::Runtime(e) => HookFailure::Runtime(anyhow::anyhow!(
                            "plugin {} body hook failed: {e:#}",
                            p.name
                        )),
                    });
                }
            }
        }
        Ok(current)
    }
}

fn validate_plugin_abi(
    store: &mut Store<HostState>,
    instance: &Instance,
    path: &Path,
) -> anyhow::Result<()> {
    let f = instance
        .get_typed_func::<(), i32>(&mut *store, "panda_abi_version")
        .map_err(|_| anyhow::anyhow!("{} missing export panda_abi_version", path.display()))?;
    let abi = f.call(&mut *store, ())?;
    if abi != PANDA_WASM_ABI_VERSION as i32 {
        anyhow::bail!(
            "{} ABI mismatch: guest={} host={}",
            path.display(),
            abi,
            PANDA_WASM_ABI_VERSION
        );
    }
    Ok(())
}

fn memory_from_caller(caller: &mut Caller<'_, HostState>) -> anyhow::Result<Memory> {
    match caller.get_export("memory") {
        Some(wasmtime::Extern::Memory(m)) => Ok(m),
        _ => anyhow::bail!("guest missing exported memory"),
    }
}

fn memory_from_store(inner: &mut PluginInner) -> anyhow::Result<Memory> {
    match inner.instance.get_export(&mut inner.store, "memory") {
        Some(wasmtime::Extern::Memory(m)) => Ok(m),
        _ => anyhow::bail!("guest missing exported memory"),
    }
}

fn read_utf8(
    caller: &Caller<'_, HostState>,
    memory: &Memory,
    ptr: i32,
    len: i32,
) -> anyhow::Result<String> {
    if ptr < 0 || len < 0 {
        anyhow::bail!("negative ptr/len from guest");
    }
    let start = ptr as usize;
    let len = len as usize;
    let end = start
        .checked_add(len)
        .ok_or_else(|| anyhow::anyhow!("ptr/len overflow"))?;
    let data = memory.data(caller);
    if end > data.len() {
        anyhow::bail!("ptr/len out of bounds");
    }
    Ok(std::str::from_utf8(&data[start..end])?.to_string())
}

fn read_bytes(
    caller: &Caller<'_, HostState>,
    memory: &Memory,
    ptr: i32,
    len: i32,
) -> anyhow::Result<Vec<u8>> {
    if ptr < 0 || len < 0 {
        anyhow::bail!("negative ptr/len from guest");
    }
    let start = ptr as usize;
    let len = len as usize;
    let end = start
        .checked_add(len)
        .ok_or_else(|| anyhow::anyhow!("ptr/len overflow"))?;
    let data = memory.data(caller);
    if end > data.len() {
        anyhow::bail!("ptr/len out of bounds");
    }
    Ok(data[start..end].to_vec())
}

fn write_guest_bytes(
    store: &mut Store<HostState>,
    memory: &Memory,
    offset: usize,
    data: &[u8],
) -> anyhow::Result<()> {
    let needed = offset
        .checked_add(data.len())
        .ok_or_else(|| anyhow::anyhow!("guest write overflow"))?;
    let page_size = 65_536usize;
    let cur_len = memory.data_size(&*store);
    if needed > cur_len {
        let to_grow = needed - cur_len;
        let pages = to_grow.div_ceil(page_size);
        memory.grow(&mut *store, pages as u64)?;
    }
    memory.write(&mut *store, offset, data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_inline_wat_add() {
        let wasm = wat::parse_str(
            r#"(module (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))"#,
        )
        .unwrap();
        let engine = Engine::default();
        let module = Module::new(&engine, wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let instance = Instance::new(&mut store, &module, &[]).unwrap();
        let add = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "add")
            .unwrap();
        assert_eq!(add.call(&mut store, (40, 2)).unwrap(), 42);
    }

    #[test]
    fn request_hook_can_set_header() {
        let wasm = wat::parse_str(
            r#"(module
                (import "panda_host" "panda_set_header"
                    (func $seth (param i32 i32 i32 i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "x-from-wasm")
                (data (i32.const 32) "enabled")
                (func (export "panda_abi_version") (result i32)
                    i32.const 0)
                (func (export "panda_on_request") (result i32)
                    i32.const 0 i32.const 11 i32.const 32 i32.const 7
                    call $seth
                    i32.const 0)
            )"#,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.wasm");
        std::fs::write(&path, wasm).unwrap();

        let engine = Engine::default();
        let plugins = PluginRuntime::load_from_directory(&engine, dir.path()).unwrap();
        assert_eq!(plugins.len(), 1);

        let runtime = PluginRuntime { engine, plugins };
        let mut headers = HeaderMap::new();
        let n = runtime.apply_request_plugins_strict(&mut headers).unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            headers
                .get("x-from-wasm")
                .and_then(|v| v.to_str().ok())
                .unwrap(),
            "enabled"
        );
    }

    #[test]
    fn body_hook_can_replace_body() {
        let wasm = wat::parse_str(
            r#"(module
                (import "panda_host" "panda_set_body"
                    (func $setb (param i32 i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "original")
                (data (i32.const 32) "sanitized")
                (func (export "panda_abi_version") (result i32)
                    i32.const 0)
                (func (export "panda_on_request_body") (param i32 i32) (result i32)
                    i32.const 32 i32.const 9
                    call $setb
                    i32.const 0)
            )"#,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("body.wasm");
        std::fs::write(&path, wasm).unwrap();

        let engine = Engine::default();
        let plugins = PluginRuntime::load_from_directory(&engine, dir.path()).unwrap();
        let runtime = PluginRuntime { engine, plugins };

        let out = runtime
            .apply_request_body_plugins_strict(b"original", 1024)
            .unwrap()
            .expect("plugin body replacement");
        assert_eq!(out, b"sanitized");
    }

    #[test]
    fn request_hook_policy_rc_is_classified() {
        let wasm = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "panda_abi_version") (result i32) i32.const 0)
                (func (export "panda_on_request") (result i32) i32.const 2)
            )"#,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("reject.wasm"), wasm).unwrap();
        let engine = Engine::default();
        let plugins = PluginRuntime::load_from_directory(&engine, dir.path()).unwrap();
        let runtime = PluginRuntime { engine, plugins };
        let mut headers = HeaderMap::new();
        let err = runtime.apply_request_plugins_strict(&mut headers).unwrap_err();
        match err {
            HookFailure::PolicyReject { code, .. } => assert_eq!(code, PolicyCode::MalformedRequest),
            HookFailure::Runtime(e) => panic!("unexpected runtime error: {e:#}"),
        }
    }
}
