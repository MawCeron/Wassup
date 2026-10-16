// Package ipc holds the types generated from proto/ipc.proto.
package ipc

//go:generate sh -c "protoc -I ../../proto --plugin=protoc-gen-go=$(go tool -n protoc-gen-go) --go_out=. --go_opt=paths=source_relative ../../proto/ipc.proto"
