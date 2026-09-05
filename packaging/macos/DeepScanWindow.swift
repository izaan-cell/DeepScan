// A minimal native macOS app window wrapping the DeepScan UI in a WKWebView.
// Replaces the earlier Chrome `--app=` hack — this is a genuinely separate
// native window (its own Dock icon, no dependency on Chrome being
// installed, real window chrome) rather than a browser with its UI
// stripped down. Compiled by build-dmg.sh and used as DeepScan.app's
// CFBundleExecutable via launcher.sh (which starts the engine + daemon,
// then execs this).

import Cocoa
import WebKit

let appURL = ProcessInfo.processInfo.environment["DEEPSCAN_URL"] ?? "http://127.0.0.1:51424"

class AppDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKUIDelegate {
    var window: NSWindow!
    var webView: WKWebView!

    func applicationDidFinishLaunching(_ notification: Notification) {
        let width: CGFloat = 1100
        let height: CGFloat = 760

        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: height),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "DeepScan"
        window.center()
        window.minSize = NSSize(width: 720, height: 480)

        let config = WKWebViewConfiguration()
        // Non-persistent: this window has no address bar, so the user has
        // no way to hard-refresh past a stale cache the way they could in
        // a real browser tab. Every launch of a rebuilt/reinstalled app
        // must always see whatever the currently-running engine actually
        // serves, never a cached response from an earlier build.
        config.websiteDataStore = .nonPersistent()
        webView = WKWebView(frame: window.contentView!.bounds, configuration: config)
        webView.autoresizingMask = [.width, .height]
        webView.navigationDelegate = self
        webView.uiDelegate = self
        // Lets Safari's Develop menu attach to inspect this exact WebView —
        // real console/network errors instead of guessing from symptoms.
        if #available(macOS 13.3, *) {
            webView.isInspectable = true
        }

        window.contentView = webView
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        setupMenu()
        loadApp()
    }

    // A minimal main menu so standard shortcuts actually work — without
    // this, an app with no nib/storyboard has no menu bar at all, so
    // there's no way to Cmd+Q normally, and (more importantly here) no
    // Cmd+R to manually retry loading if the automatic retries in
    // retryLoad() below ever fall behind a slow-starting engine.
    func setupMenu() {
        let mainMenu = NSMenu()

        let appMenuItem = NSMenuItem()
        mainMenu.addItem(appMenuItem)
        let appMenu = NSMenu()
        appMenu.addItem(NSMenuItem(title: "Quit DeepScan", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
        appMenuItem.submenu = appMenu

        let viewMenuItem = NSMenuItem()
        mainMenu.addItem(viewMenuItem)
        let viewMenu = NSMenu(title: "View")
        let reloadItem = NSMenuItem(title: "Reload", action: #selector(reload), keyEquivalent: "r")
        reloadItem.target = self
        viewMenu.addItem(reloadItem)
        viewMenuItem.submenu = viewMenu

        NSApp.mainMenu = mainMenu
    }

    @objc func reload() {
        retryCount = 0
        loadApp()
    }

    func loadApp() {
        guard let url = URL(string: appURL) else { return }
        var request = URLRequest(url: url)
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        webView.load(request)
    }

    // The engine can take a while to bind its port after this window opens
    // (launcher.sh only waits for engine.lock, not the HTTP listener
    // specifically, and cold model loading from disk can be slow) — retry
    // indefinitely rather than capping out and leaving the window frozen
    // on a dead error page with no address bar or reload button to
    // recover from (that's what the earlier capped-at-20-tries version
    // did, and there was no way to notice a slow-but-not-crashed engine).
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        retryLoad()
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        retryLoad()
    }

    var retryCount = 0
    func retryLoad() {
        retryCount += 1
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
            self?.loadApp()
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        return true
    }

    // WKWebView has no built-in file-picker support for <input type="file">
    // clicks — Safari/Chrome implement this internally, but a bare WKWebView
    // silently does nothing on click unless the host app implements this
    // delegate method itself. This is what "drop zone" clicks needed.
    func webView(
        _ webView: WKWebView,
        runOpenPanelWith parameters: WKOpenPanelParameters,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping ([URL]?) -> Void
    ) {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = parameters.allowsMultipleSelection
        panel.begin { result in
            completionHandler(result == .OK ? panel.urls : nil)
        }
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
