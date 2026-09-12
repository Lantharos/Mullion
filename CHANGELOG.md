# Sabine 0.27

- Windows OSR connections switch accepted sockets to blocking reads before authentication,
  preventing startup disconnections when Chromium pauses between messages.
- Windows prepares Chromium resource packs beside its runtime DLLs as well as ICU data.
- Chromium uses D3D11 WARP when the Windows renderer selects a software adapter, including
  virtual machines without GPU acceleration. Hardware adapters retain D3D11, and both paths
  keep shared-texture rendering.
- Windows installer checks require the installed app to present a Chromium frame and keep
  its OSR connection alive. Missing resource packs and connection recovery fail validation.

# Sabine 0.26

- Previously downloaded app updates are only offered or installed while their version is newer
  than the installed app. Manually installing a newer app makes an older pending update ineligible.
- Windows prepares ICU data beside the Chromium library before launch, fixing startup crashes
  reporting an invalid ICU data file descriptor.
- Host protocol checks allow up to 30 seconds for cold starts on slower machines and virtual
  machines. Successful checks return immediately.
- Windows package and app release checks require Chromium to render and verify a probe page,
  in addition to checking installation and background-service startup.
- App release workflows accept the application's Rust toolchain and install the dependencies
  needed to build the matching CLI and publish signed update manifests.

Windows MSI cancellation, installation, repair, browser rendering, and uninstall passed on a
native Windows runner. Workspace builds, tests, strict Clippy, and Windows cross-compilation
are required before publishing. The minimum supported app build remains 23.

# Sabine 0.25

Apps built before Sabine 0.23 must be rebuilt before using this shared system. The minimum
supported app build is 23 because the bridge authorization and host protocol changed. Existing app registrations
remain available for updates. macOS releases support Apple Silicon; Intel Mac targets have been
removed. The minimum Rust version is 1.90.

- Windows EXE and MSI installers prepare the shared runtime inside the installer wizard, report
  setup failures, support cancellation and repair, and roll back failed setup. EXE upgrades remove
  obsolete application files while preserving user-created files.
- Windows background processes run without console windows. Native process ownership, shutdown,
  bounded diagnostic logs, and visible startup errors make failed launches easier to diagnose.
- Chromium runs with its sandbox on Linux, Windows, and macOS. Native bridge commands are bound to
  authorized documents, and unsupported permission requests resolve with a denial. Linux provides
  an AppArmor profile command for systems that restrict unprivileged namespaces.
- Rendering follows the active display's refresh rate. Bounded paint queues, shared GPU instance
  lifetime, frame retirement, and device-loss recovery reduce retained resources and idle work.
  Hidden-window hibernation releases Chromium and per-window GPU resources.
- Runtime and application updates use operating-system locks, recoverable publication, validated
  archive extraction, and bounded resumable downloads. App updates remain available when an older
  app cannot use the current shared system. Daemon ownership is serialized across concurrent starts.
- Linux packages have private application payloads, native dependency metadata, and corrected
  desktop registration. Icons are generated in process, including SVG, Windows ICO, and macOS ICNS.
- Apple Silicon bundles use sandbox-compatible CEF launch bundles without modifying the original
  host bundle. URL and document delivery works at startup and in an existing application.
- Guest shortcut, wheel, and download options agree across Rust and JavaScript. The typed package
  includes cancellation, request deadlines, open-URL delivery, and native Save-dialog options.
  Chromium and Firefox native messaging use their respective manifest and registration formats.

Validation covers workspace builds, tests, strict Clippy, Windows cross-compilation, native CEF
rendering and shutdown on all three desktop platforms, Linux package formats, macOS bundles, and
Windows installer cancellation, install, repair, launch, and uninstall. EXE upgrade checks also
verify user-file preservation. Linux runtime checks cover guest lifetime, recovery, sandboxing,
input forwarding, bridge permissions, downloads, hibernation, and concurrent daemon startup.

Native screen-reader integration is not implemented. Linux and macOS browser dialogs use the
system's default ownership. Physical Windows/macOS IME and hybrid-GPU behavior still need device
testing; CI uses hosted machines. macOS distribution signing and notarization remain the app
publisher's responsibility.

The v0.23 and v0.24 tags remain available for source history; neither produced a published release.
Version 0.25 includes the external-host AppArmor correction caught during v0.23 validation and uses
the tested Linux CLI to generate release metadata, avoiding the redundant build that stopped v0.24
publication. Use v0.25 when updating application dependencies.
