package main

import "unsafe"

//go:wasmimport panda_host panda_set_header
func pandaSetHeader(namePtr int32, nameLen int32, valuePtr int32, valueLen int32)

//go:wasmimport panda_host panda_set_body
func pandaSetBody(ptr int32, length int32)

var headerName = []byte("x-panda-plugin")
var headerValue = []byte("tinygo-sample")

//export panda_abi_version
func panda_abi_version() int32 {
	return 0
}

//export panda_on_request
func panda_on_request() int32 {
	pandaSetHeader(
		int32(uintptr(unsafe.Pointer(&headerName[0]))),
		int32(len(headerName)),
		int32(uintptr(unsafe.Pointer(&headerValue[0]))),
		int32(len(headerValue)),
	)
	return 0
}

//export panda_on_request_body
func panda_on_request_body(ptr int32, length int32) int32 {
	// v0 sample: no-op allow.
	_ = ptr
	_ = length
	return 0
}

func main() {}
