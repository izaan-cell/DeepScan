package main

import (
	"os"
	"path/filepath"
	"strings"

	"github.com/joho/godotenv"
)

// loadEnv loads DeepScan's env files with the same precedence used by the
// Rust engine (see rust-engine/src/config.rs): in development, .env.dev
// first, then .env for personal overrides of anything .env.dev doesn't set;
// godotenv never overwrites a variable already present in the process
// environment, so more specific files must load first.
func loadEnv() {
	env := os.Getenv("DEEPSCAN_ENV")
	if env == "" {
		env = "development"
	}
	if env == "development" {
		_ = godotenv.Load(repoRoot(".env.dev"))
	}
	_ = godotenv.Load(repoRoot(".env"))
}

// repoRoot resolves a path relative to the repo root (one level up from
// go-daemon/), so `go run .` works whether invoked from the repo root or
// from within go-daemon/.
func repoRoot(name string) string {
	if _, err := os.Stat(name); err == nil {
		return name
	}
	return filepath.Join("..", name)
}

func watchRootsFromEnv() []string {
	raw := os.Getenv("DEEPSCAN_WATCH_ROOTS")
	if raw == "" {
		return defaultWatchRoots()
	}
	var roots []string
	for _, part := range strings.Split(raw, ",") {
		roots = append(roots, resolvePath(strings.TrimSpace(part)))
	}
	return roots
}

// resolvePath expands a leading "~" to the home directory, and resolves a
// "./"-relative dev path (e.g. DEEPSCAN_WATCH_ROOTS=./sample-data in
// .env.dev) against the repo root rather than whatever directory the
// daemon happened to be launched from.
func resolvePath(path string) string {
	if strings.HasPrefix(path, "~") {
		home, err := userHomeDir()
		if err != nil {
			return path
		}
		return filepath.Join(home, strings.TrimPrefix(path, "~"))
	}
	if strings.HasPrefix(path, "./") || strings.HasPrefix(path, "../") {
		return repoRoot(path)
	}
	return path
}
