//! Scenario YAML profiles aligned with `docs/panda_scenarios_summary.md`.
//! Ensures representative configs parse and `effective_*` helpers behave as documented.

use panda_config::PandaConfig;

#[test]
fn scenario_a_ai_gateway_only_no_mcp() {
    let cfg = PandaConfig::from_yaml_str(
        r#"listen: '127.0.0.1:0'
default_backend: 'http://127.0.0.1:5023'
routes:
  - path_prefix: /v1/chat
    backend_base: 'https://api.openai.com/v1'
mcp:
  enabled: false
"#,
    )
    .unwrap();
    assert!(!cfg.mcp.enabled);
    assert_eq!(
        cfg.effective_backend_base("/v1/chat/completions"),
        "https://api.openai.com/v1"
    );
}

#[test]
fn scenario_b_mcp_and_egress_no_routes() {
    let cfg = PandaConfig::from_yaml_str(
        r#"listen: '127.0.0.1:0'
default_backend: 'http://127.0.0.1:5023'
api_gateway:
  ingress:
    enabled: true
  egress:
    enabled: true
    corporate:
      default_base: 'http://127.0.0.1:18081'
    allowlist:
      allow_hosts: ['127.0.0.1:18081']
      allow_path_prefixes: ['/allowed']
mcp:
  enabled: true
  advertise_tools: true
  servers:
    - name: corpapi
      enabled: true
      http_tool:
        path: /allowed/p
        method: GET
        tool_name: t
"#,
    )
    .unwrap();
    assert!(cfg.mcp.enabled);
    assert!(cfg.api_gateway.egress.enabled);
}

#[test]
fn scenario_c_routes_mcp_advertise_global_false_route_true() {
    let cfg = PandaConfig::from_yaml_str(
        r#"listen: '127.0.0.1:0'
default_backend: 'http://127.0.0.1:5023'
routes:
  - path_prefix: /v1/chat
    backend_base: 'http://127.0.0.1:5023'
    mcp_advertise_tools: true
mcp:
  enabled: true
  advertise_tools: false
  servers:
    - name: a
"#,
    )
    .unwrap();
    assert!(!cfg.effective_mcp_advertise_tools_for_path("/v1/embeddings"));
    assert!(cfg.effective_mcp_advertise_tools_for_path("/v1/chat/completions"));
}

#[test]
fn scenario_e_longest_prefix_embeddings_uses_default_backend() {
    let cfg = PandaConfig::from_yaml_str(
        r#"listen: '127.0.0.1:0'
default_backend: 'http://127.0.0.1:9'
routes:
  - path_prefix: /v1/chat
    backend_base: 'http://127.0.0.1:7'
mcp:
  enabled: false
"#,
    )
    .unwrap();
    assert_eq!(
        cfg.effective_backend_base("/v1/chat/completions"),
        "http://127.0.0.1:7"
    );
    assert_eq!(
        cfg.effective_backend_base("/v1/embeddings"),
        "http://127.0.0.1:9"
    );
    assert_eq!(cfg.effective_adapter_provider("/v1/embeddings"), "openai");
}

#[test]
fn scenario_f_per_route_adapter_type_overrides_global() {
    let cfg = PandaConfig::from_yaml_str(
        r#"listen: '127.0.0.1:0'
default_backend: 'http://127.0.0.1:1'
adapter:
  provider: openai
routes:
  - path_prefix: /v1/chat
    backend_base: 'http://127.0.0.1:2'
    type: anthropic
mcp:
  enabled: false
"#,
    )
    .unwrap();
    assert_eq!(cfg.effective_adapter_provider("/v1/chat/foo"), "anthropic");
    assert_eq!(cfg.effective_adapter_provider("/v1/embeddings"), "openai");
}

#[test]
fn scenario_d_repo_root_panda_yaml_parses() {
    let yaml = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../panda.yaml"));
    let cfg = PandaConfig::from_yaml_str(yaml).expect("repo panda.yaml must parse");
    assert!(cfg.mcp.enabled);
    assert!(cfg.api_gateway.ingress.enabled);
    assert!(cfg.api_gateway.egress.enabled);
    assert!(!cfg.mcp.advertise_tools);
    assert!(!cfg.effective_mcp_advertise_tools_for_path("/v1/embeddings"));
    assert!(cfg.effective_mcp_advertise_tools_for_path("/v1/chat/completions"));
}
