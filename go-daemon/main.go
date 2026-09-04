// Package main is the DeepScan background daemon: it watches configured
// directories for changes, streams events to the Rust engine over gRPC, runs
// the system tray icon, and provides the native "reveal in Finder/Explorer"
// hooks the UI calls when a user picks a search result.
package main

import (
	"context"
	"log"
	"runtime"

	"github.com/getlantern/systray"
)

func main() {
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

	// Directories the user has added via the UI — persisted separately;
	// hardcoded home dir here as the scaffold default.
	for _, root := range defaultWatchRoots() {
		if err := watcher.AddRecursive(root); err != nil {
			log.Printf("warning: could not watch %s: %v", root, err)
		}
	}

	go watcher.Run(ctx)

	systray.Run(onTrayReady(watcher), onTrayExit(cancel))
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
