#!/usr/bin/env python3
import json
import os
from pathlib import Path
import plistlib
import signal
import subprocess
import sys
import tempfile
import time

if sys.platform != "darwin":
    raise SystemExit("This check requires macOS")

repository = Path(__file__).resolve().parent.parent
build = subprocess.run(
    ["cargo", "build", "-p", "sabine", "--message-format=json"],
    cwd=repository, capture_output=True, text=True, check=True,
)
artifacts = {}
for line in build.stdout.splitlines():
    message = json.loads(line)
    if message.get("reason") == "compiler-artifact":
        for filename in message["filenames"]:
            if filename.endswith(".rlib"):
                artifacts[message["target"]["name"]] = filename

with tempfile.TemporaryDirectory(prefix="sabine-url-check-") as temporary:
    root = Path(temporary)
    bundle = root / "Sabine URL Check.app"
    macos = bundle / "Contents/MacOS"
    macos.mkdir(parents=True)
    received = root / "received.jsonl"
    pidfile = root / "pid"
    document = root / "Document with spaces.txt"
    document.write_text("Open this document through LaunchServices")
    source = root / "main.rs"
    source.write_text(r'''
use std::{io::Write, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
use winit::{application::ApplicationHandler, event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop}, window::WindowId};
type EventQueue = crossbeam_channel::Sender<sabine_platform::PlatformEvent>;
#[path = "@MODULE@"] mod open_urls;
struct App { done: Arc<AtomicBool> }
impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, events: &dyn ActiveEventLoop) { events.set_control_flow(ControlFlow::Wait); }
    fn proxy_wake_up(&mut self, events: &dyn ActiveEventLoop) {
        if self.done.load(Ordering::Acquire) { events.exit(); }
    }
    fn window_event(&mut self, _: &dyn ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
fn main() {
    std::fs::write("@PID@", std::process::id().to_string()).unwrap();
    let event_loop = EventLoop::new().unwrap();
    let (sender, receiver) = crossbeam_channel::unbounded();
    let _events = open_urls::OpenUrlEvents::install(sender).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let completed = done.clone();
    let proxy = event_loop.create_proxy();
    let worker = std::thread::spawn(move || {
        let mut output = std::fs::OpenOptions::new().create(true).append(true).open("@OUTPUT@").unwrap();
        let mut success = true;
        for _ in 0..2 {
            match receiver.recv_timeout(Duration::from_secs(20)) {
                Ok(sabine_platform::PlatformEvent::OpenUrls(urls)) => {
                    writeln!(output, "{}", serde_json::to_string(&urls).unwrap()).unwrap();
                    output.flush().unwrap();
                },
                _ => { success = false; break; }
            }
        }
        completed.store(true, Ordering::Release);
        proxy.wake_up();
        success
    });
    event_loop.run_app(&mut App { done }).unwrap();
    assert!(worker.join().unwrap(), "LaunchServices did not deliver both URL events");
}
'''.replace("@MODULE@", str(repository / "crates/sabine/src/desktop/macos/open_urls.rs"))
        .replace("@PID@", str(pidfile)).replace("@OUTPUT@", str(received)))
    command = ["rustc", "--edition=2024", str(source), "-o", str(macos / "url-check"),
               "-L", f"dependency={repository}/target/debug/deps"]
    for dependency in ["winit", "objc2", "objc2_app_kit", "objc2_foundation",
                       "crossbeam_channel", "sabine_platform", "serde_json"]:
        command.extend(["--extern", f"{dependency}={artifacts[dependency]}"])
    subprocess.run(command, check=True)
    with (bundle / "Contents/Info.plist").open("wb") as output:
        plistlib.dump({
            "CFBundleIdentifier": f"dev.sabine.url-check.p{os.getpid()}",
            "CFBundleExecutable": "url-check",
            "CFBundleName": "Sabine URL Check",
            "CFBundlePackageType": "APPL",
            "CFBundleVersion": "1.0",
            "LSUIElement": True,
            "CFBundleURLTypes": [{"CFBundleURLSchemes": ["sabine-url-check"]}],
            "CFBundleDocumentTypes": [{"CFBundleTypeExtensions": ["txt"], "CFBundleTypeRole": "Viewer"}],
        }, output)

    def wait_for_count(count):
        deadline = time.monotonic() + 20
        while True:
            lines = received.read_text().splitlines() if received.exists() else []
            if len(lines) >= count:
                return [json.loads(line) for line in lines]
            if time.monotonic() >= deadline:
                raise RuntimeError(f"macOS did not deliver URL event {count}")
            time.sleep(0.05)

    try:
        subprocess.run(["open", "-n", "-g", "-a", str(bundle), "sabine-url-check://cold-start"], check=True)
        assert wait_for_count(1) == [["sabine-url-check://cold-start"]]
        subprocess.run(["open", "-g", "-a", str(bundle), str(document)], check=True)
        assert wait_for_count(2) == [["sabine-url-check://cold-start"], [document.as_uri()]]
        print("macOS delivered the cold-start URL and subsequent document to the application delegate")
    finally:
        if pidfile.exists():
            pid = int(pidfile.read_text())
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
