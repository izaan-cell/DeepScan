package main

import (
	"fmt"
	"os/exec"
	"runtime"
)

// RevealInOS highlights path in the native file browser: Finder on macOS,
// File Explorer on Windows. Called when the UI's "System Hook Actions"
// button is pressed on a search result.
// Start(), not Run() — Run() blocks the caller until the child process
// fully exits, which only matters if something needs its exit code. It
// doesn't here: revealing a file in Finder/Explorer is fire-and-forget,
// and waiting on it needlessly stalls whatever called this (see
// rust-engine/src/oshooks.rs's mirrored fix — that one was live-wired to
// the HTTP handler and was actually stalling the whole engine).
func RevealInOS(path string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", "-R", path).Start()
	case "windows":
		// explorer.exe returns a non-zero exit code on success in some
		// versions; ignore the error value deliberately and only surface
		// a real failure to start the process.
		return exec.Command("explorer.exe", "/select,"+path).Start()
	default:
		return fmt.Errorf("RevealInOS: unsupported platform %s", runtime.GOOS)
	}
}
