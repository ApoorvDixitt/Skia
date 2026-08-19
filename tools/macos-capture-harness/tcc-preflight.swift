import CoreGraphics
import Foundation
// Preflight only — this API checks status WITHOUT triggering a permission prompt.
let granted = CGPreflightScreenCaptureAccess()
print("CGPreflightScreenCaptureAccess() = \(granted)")
print("responsible process: \(ProcessInfo.processInfo.processName)")
