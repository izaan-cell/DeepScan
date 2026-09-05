package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

type engineLock struct {
	Port int `json:"port"`
	PID  int `json:"pid"`
}

// readEngineLock reads engine.lock, written by the Rust engine on startup
// (see rust-engine/src/main.rs's write_lockfile, and config.rs for the
// matching DEEPSCAN_DATA_DIR resolution this mirrors), and returns its
// loopback address.
func readEngineLock() (string, error) {
	raw, err := os.ReadFile(filepath.Join(dataDir(), "engine.lock"))
	if err != nil {
		return "", fmt.Errorf("engine.lock not found — is the DeepScan engine running? %w", err)
	}
	var lock engineLock
	if err := json.Unmarshal(raw, &lock); err != nil {
		return "", err
	}
	return fmt.Sprintf("127.0.0.1:%d", lock.Port), nil
}

func dialEngine(addr string) (*grpc.ClientConn, error) {
	return grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
}

func userHomeDir() (string, error) {
	return os.UserHomeDir()
}
