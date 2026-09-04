package main

import (
	"fmt"
	"os/exec"
	"runtime"
)

// RevealInOS highlights path in the native file browser: Finder on macOS,
// File Explorer on Windows. Called when the UI's "System Hook Actions"
// button is pressed on a search result.
func RevealInOS(path string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", "-R", path).Run()
	case "windows":
		// explorer.exe returns a non-zero exit code on success in some
		// versions; ignore the error value deliberately and only surface
		// a real failure to start the process.
		cmd := exec.Command("explorer.exe", "/select,"+path)
		_ = cmd.Run()
		return nil
	default:
		return fmt.Errorf("RevealInOS: unsupported platform %s", runtime.GOOS)
	}
}
