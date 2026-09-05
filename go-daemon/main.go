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
	var scanRoots []string
	for _, root := range watchRootsFromEnv() {
		if err := watcher.AddRecursive(root); err != nil {
			log.Printf("warning: could not watch %s: %v", root, err)
			continue
		}
		scanRoots = append(scanRoots, root)
	}

	// One goroutine running all initial scans back-to-back, not one
	// goroutine per root — the engine embeds one file at a time behind a
	// single mutex regardless (see rust-engine/src/service.rs), so 7
	// roots scanning "concurrently" was really 7 goroutines constantly
	// re-contending for that same lock with no throughput benefit, and in
	// practice this starved a live /api/search request out for minutes at
	// a time on a large enough initial index. Scanning one root fully
	// before starting the next means a search only ever has to wait
	// behind a single in-flight index_file call, not seven.
	go func() {
		for _, root := range scanRoots {
			// fsnotify only reports changes from this point forward —
			// files that already exist need this one-shot walk to ever
			// get indexed.
			triggerInitialScan(ctx, conn, root)
		}
	}()

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

// The default scope: every common place a user's own files actually live,
// not just Desktop/Documents (which missed real content in ~/Downloads
// entirely — an image search for a file the user had just downloaded found
// nothing there and surfaced unrelated matches from elsewhere in the index
// instead). Applications is included so app bundles show up too, but see
// isSoftwareProjectDir — the walk never descends *into* a .app, only notes
// its presence.
func defaultWatchRoots() []string {
	home, err := userHomeDir()
	if err != nil {
		return nil
	}
	if runtime.GOOS == "windows" {
		return []string{
			home + `\Desktop`, home + `\Documents`, home + `\Downloads`,
			home + `\Pictures`, home + `\Videos`, home + `\Music`,
		}
	}
	if runtime.GOOS == "darwin" {
		return []string{
			home + "/Desktop", home + "/Documents", home + "/Downloads",
			home + "/Pictures", home + "/Movies", home + "/Music",
			"/Applications",
		}
	}
	return []string{home + "/Desktop", home + "/Documents", home + "/Downloads"}
}
