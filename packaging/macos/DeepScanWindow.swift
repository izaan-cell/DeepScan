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
        requestFolderAccess()
        loadApp()
    }

    // macOS only shows the "DeepScan would like to access files in your
    // Desktop folder" consent dialog the first time a *foreground, visible*
    // process belonging to this app actually touches one of these
    // protected folders — a background helper (the Go daemon/Rust engine,
    // spawned headless by launcher.sh) reading the same folder gets
    // silently denied with no dialog at all, which is what was actually
    // happening: every real scan of Desktop/Documents/Downloads failed
    // quietly with "operation not permitted", and there was never a prompt
    // to say yes to. Doing one deliberate read from here — the actual
    // NSApplication the user sees and has just brought to the front — is
    // what gets TCC to attribute the request correctly and ask.
    func requestFolderAccess() {
        let fm = FileManager.default
        for folder in ["Desktop", "Documents", "Downloads"] {
            let path = (NSHomeDirectory() as NSString).appendingPathComponent(folder)
            DispatchQueue.global(qos: .utility).async {
                _ = try? fm.contentsOfDirectory(atPath: path)
            }
        }
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

        // Cmd+C/V/X/A do nothing anywhere in the app without this — AppKit
        // only routes a key equivalent to the first responder (here, the
        // WKWebView, which implements copy:/paste:/etc. itself) if some
        // menu item in the menu bar actually claims that key equivalent.
        // There's no nib building this for free, so it has to exist here.
        let editMenuItem = NSMenuItem()
        mainMenu.addItem(editMenuItem)
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(NSMenuItem(title: "Undo", action: Selector(("undo:")), keyEquivalent: "z"))
        let redoItem = NSMenuItem(title: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redoItem.keyEquivalentModifierMask = [.command, .shift]
        editMenu.addItem(redoItem)
        editMenu.addItem(NSMenuItem.separator())
        editMenu.addItem(NSMenuItem(title: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x"))
        editMenu.addItem(NSMenuItem(title: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c"))
        editMenu.addItem(NSMenuItem(title: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v"))
        editMenu.addItem(NSMenuItem(title: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a"))
        editMenuItem.submenu = editMenu

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
