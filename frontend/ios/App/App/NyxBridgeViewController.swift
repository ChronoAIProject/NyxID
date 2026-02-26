import UIKit
import Capacitor

// ── Custom CAPBridgeViewController ──
// Eliminates white flash between native LaunchScreen
// and WebView first paint by forcing dark background
// on the WKWebView before any HTML content loads.

class NyxBridgeViewController: CAPBridgeViewController {

    override func viewDidLoad() {
        super.viewDidLoad()

        let bg = UIColor(red: 6/255, green: 6/255, blue: 10/255, alpha: 1) // #06060A
        view.backgroundColor = bg
        webView?.isOpaque = false
        webView?.backgroundColor = bg
        webView?.scrollView.backgroundColor = bg
    }
}
