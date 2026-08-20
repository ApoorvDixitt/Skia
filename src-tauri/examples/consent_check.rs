// Diagnostic: exercises the exact consent code path the app runs.
// Run from a terminal whose responsible app already holds a mic grant:
// Granted proves the objc2 plumbing works; an error names what broke.
fn main() {
    match skia_lib::audio::ensure_microphone() {
        Ok(()) => println!("consent path OK: ensure_microphone() -> Granted"),
        Err(e) => println!("consent path FAILED: {e}"),
    }
}
