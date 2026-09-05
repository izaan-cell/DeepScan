// Package main is the DeepScan background daemon: it watches configured
// directories for changes, streams events to the Rust engine over gRPC, and
// provides the native "reveal in Finder/Explorer" hooks the UI calls when a
// user picks a search result.
package main

import (
	"context"
	"log"
	"os"
	"os/signal"
	"runtime"
	"syscall"
)

func main() {
	loadEnv()

	engineAddr, err := readEngineLock()
	if err != nil {
		log.Fatalf("could not find running DeepScan engine: %v", err)
	}

	conn, err := dialEngine(engineAddr)
	if err != nil {
		log.Fatalf("failed to connect to engine at %s: %v", engineAddr, err)
	}
	defer conn.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	watcher, err := NewWatcher(conn)
	if err != nil {
		log.Fatalf("failed to start filesystem watcher: %v", err)
	}

	// Directories to watch — DEEPSCAN_WATCH_ROOTS from .env.dev/.env, or
	// the user's Documents/Desktop by default (see config.go).
	for _, root := range watchRootsFromEnv() {
		if err := watcher.AddRecursive(root); err != nil {
			log.Printf("warning: could not watch %s: %v", root, err)
			continue
		}
		// fsnotify only reports changes from this point forward — files
		// that already exist need this one-shot walk to ever get indexed.
		go triggerInitialScan(ctx, conn, root)
	}

	go watcher.Run(ctx)

	// No system tray for now — getlantern/systray's native macOS loop
	// reliably SIGABRTs when run from this daemon's build/signing setup
	// (a low-level cgo crash, unrecoverable via Go's recover()), taking the
	// whole daemon down with it. A tray icon isn't worth losing file
	// watching over; revisit as its own properly-scoped fix later.
	sigs := make(chan os.Signal, 1)
	signal.Notify(sigs, syscall.SIGINT, syscall.SIGTERM)
	<-sigs
	cancel()
}

func defaultWatchRoots() []string {
	home, err := userHomeDir()
	if err != nil {
		return nil
	}
	if runtime.GOOS == "windows" {
		return []string{home + `\Documents`, home + `\Desktop`}
	}
	return []string{home + "/Documents", home + "/Desktop"}
}
