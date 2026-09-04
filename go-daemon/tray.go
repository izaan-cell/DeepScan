package main

import "github.com/getlantern/systray"

// onTrayReady wires up the minimal system tray menu: status + quit.
// Icon bytes are loaded from the monochrome DeepScan glyph at build time.
func onTrayReady(w *Watcher) func() {
	return func() {
		systray.SetTitle("DeepScan")
		systray.SetTooltip("DeepScan — local AI search daemon")

		mStatus := systray.AddMenuItem("Indexing active", "")
		mStatus.Disable()
		systray.AddSeparator()
		mQuit := systray.AddMenuItem("Quit DeepScan", "Stop the background daemon")

		go func() {
			<-mQuit.ClickedCh
			systray.Quit()
		}()
	}
}

func onTrayExit(cancel func()) func() {
	return func() {
		cancel()
	}
}
