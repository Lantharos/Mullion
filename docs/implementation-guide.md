# Sabine implementation guide

This document describes the current architecture and the boundaries contributors should preserve.

## Release and update model

Sabine publishes the CLI, service, daemon, and prebuilt CEF host as one `vMAJOR.BUILD` release train.
Cargo and npm represent the same release with a trailing patch zero because their package formats
require three components. Build numbers advance for ordinary releases; a fundamental contract break
advances the major. A
platform system archive and `sabine-release.json` are immutable release inputs. Metadata is signed
with Ed25519; bootstrap verifies the embedded trust key and artifact SHA-256, then installs the
binaries into a versioned directory. The active pointer changes atomically and the previous version
is retained as the rollback installation until the next successful upgrade. The old binary supervises a single-daemon
handoff; failed startup restores the previous pointer and daemon, records the failed release, and
applies an exponential retry delay. A damaged active installation is silently replaced from signed
release metadata. The host is compiled against CEF Stable API 133, so the runtime service can
independently install newer compatible CEF builds. Apps negotiate Sabine behavior and capabilities
and never request a Chromium version.

Registry writes, runtime installation, and shared-system installation use operating-system file
locks. A crashed owner releases its lock immediately, and an active operation's lock is never
stolen because of its age. File replacements preserve the previous destination until the rename
succeeds, including on Windows.
Runtime and shared-system directory replacements retain the previous directory until validated
staging is published. The next installation lock restores an interrupted replacement or cleans up
its backup. Service startup recovers a missing active directory before attempting a network repair.
Runtime lease creation shares a short mutation lock with replacement and pruning, so an app cannot
start using a runtime while it is being removed; downloads do not hold this mutation lock.

Routine Sabine and app updates become eligible 24 hours after publication plus a stable
per-installation rollout offset between zero and six hours. This spreads load and leaves time to
withdraw a bad release without repeatedly prompting users. System releases are also kept off the
GitHub `latest` channel for their first 24 hours, which protects clients running an updater from
before this policy existed; hourly automation promotes the newest eligible immutable release.
A newer app that declares a newer Sabine build bypasses both gates by fetching that build's signed
versioned manifest: its bundled bootstrap stages the required system, hands off the daemon, and only
then registers the app. System release metadata declares its current build and the oldest app build
it accepts. Apps older than that floor are removed from shared registration and receive a native
incompatibility notice. Legacy registrations without compatibility metadata remain accepted until
they next register with a current Sabine build.

App releases are separate from Sabine releases. `[updates]` in `Sabine.toml` identifies a GitHub
repository or HTTPS manifest endpoint. Sabine's reusable GitHub Actions workflow builds native
artifacts and emits a signed `sabine-update.json`. The service verifies its configured app public
key, then validates app id, channel, exact platform/package target, version, artifact URL, and
SHA-256 before staging it. Each app has its own Actions signing secret; only the public key ships in
the app.

Writable managed archives activate side by side in the background. The original executable acts as
a stable bootstrap and forwards the next launch to the executable recorded in the service registry.
Native packages are also downloaded in the background, but activation is foreground: the next app
launch offers Install or Later, exits after acceptance, invokes MSI, DMG replacement, AppImage
replacement, or `pkexec` for deb/rpm, and relaunches after a successful installer exit. Store-owned
installations remain owned by the store.

CEF installation is a separate release stream. The daemon downloads the newest compatible Standard
runtime into a side-by-side directory, initializes it with the installed host using a headless health
probe, and only then leaves it selectable. Failed runtimes receive an unusable marker and resolution
returns to the previous runtime. Markers are scoped to the probe version so a fixed probe retries a
runtime automatically. Each running host owns a process lease; pruning keeps the newest two runtimes
and never removes a leased directory. Offline bundles include a real runtime and the complete system
bootstrap, but first launch adopts the embedded binaries into the normal managed layout so both
streams continue updating.

## Process model

A Sabine app can run in three modes from the same executable:

1. The normal app process configures `SabineWindow`, native services, bridge commands, and policy.
2. The bootstrap child shows a native progress window while the shared service is downloaded (if
   needed) and the Chromium runtime is installed. App installers ship app code plus this bootstrap so
   the first launch prepares the machine for every future Sabine app.
3. The OSR native-host child owns the winit window, wgpu renderer, input, composition, and CEF helper.

Apps enter through `SabineWindow::main`. It dispatches internal child modes, builds the app window,
launches it, and owns the process wait. `SabineWindow::launch` remains available when an application
needs to integrate Sabine into an existing native process loop.

Custom entry points that initialize an argument parser, logger, or application runtime should
dispatch Sabine's internal children before doing any of that work:

```rust
fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if sabine::dispatch_host_mode_from_args(&args) {
        return;
    }

    // Parse application arguments and initialize the app here.
}
```

The root dispatcher covers both OSR-host and runtime-bootstrap children. It returns `false` for an
ordinary app invocation without initializing Sabine. `SabineWindow::main` calls it automatically.

The CEF helper is built from the C++ source embedded in the `sabine` crate. It always enables
windowless rendering and always creates `SabineOsrHandler`; there is no windowed CEF handler.

### App identity and windows

Every launch requires a non-empty `app_id`. `SabineWindow::main` loads identity and production web
configuration from the framework-managed `Sabine.toml` before applying the app's Rust builder, so
explicit Rust settings remain authoritative. That id selects the CEF profile directory and the
private IPC directory under `$XDG_RUNTIME_DIR/sabine/<app_id>/` (mode `0700`, sockets `0600`).

- **Different `app_id`**: separate CEF processes and profiles; no handoff between apps.
- **Same `app_id`**: windows share one CEF browser process (process-singleton handoff). Each window
  still has its own OSR host and authenticated Unix socket. Paint, input, and bridge
  request/response/events all travel on that per-window socket so invokes reach the owning app
  process. Closing one window closes only that browser; CEF quits when the last OSR handler is gone.
  Separate top-level launches therefore close independently while still being able to talk through
  the shared profile/CEF process when the app wants that.
- **Same-process multi-window**: call `SabineProcess::open_window` to spawn another OSR host with
  the same handlers and `app_id` without a second top-level `wait()` island. Use
  `close_window(WindowId)` to tear down one surface; `wait()` returns only after every OSR window
  has exited. If the parent app process exits, its OSR children are terminated; closing a child
  window does not exit the parent.
- **Second OS launch**: optional `single_instance` still focuses an existing process when enabled.
  Without it, a second launch of the same app uses CEF handoff plus a new per-window socket.
  After exit-24 handoff the secondary host waits up to 15s for the primary CEF process to connect.

OSR authentication uses a first-line token plus same-UID `SO_PEERCRED` checks on Unix. The token
is written to a `0600` file beside the socket and referenced by `--sabine-osr-token-file=` on the
CEF command line (path is not secret; this survives process-singleton handoff). `SABINE_OSR_TOKEN`
remains an optional fallback. Stale-socket cleanup uses a same-UID health-probe line that listeners
recognize separately from authenticated CEF connections. Child OSR/CEF processes set
`PR_SET_PDEATHSIG` on Linux so OSR hosts do not outlive a crashed parent.
Closing a window stops browser recovery immediately. Transport disconnect forces closure of
that window's browser, while other windows sharing the CEF process remain alive. A browser
that cannot connect to its native window closes instead of remaining invisible.
Each native connection owns its socket reader and bounded paint queue. Closing or hibernating
a window cancels blocked reads and queue writes, then removes its endpoint. Native timers
return to an idle wait after firing; hidden windows remain suspended across focus changes.
A failed browser connection gets at most three recovery attempts in 60 seconds, with increasing
delays. Initial connection is limited to 30 seconds; profile handoff remains limited to 15 seconds.
Terminal failures produce persistent diagnostics, a native error notice, and a nonzero exit.
Renderer crashes reload the affected browser at most three times in 60 seconds. The fourth
closes the failed browser; a main-page failure becomes a native error. Apps can observe
`runtime.renderer-crashed` events with `guestId`, `code`, and `recovering` fields.
Windows owns each OSR host through a kill-on-close job, requests graceful native-window
closure, and bounds the wait before terminating a stalled host. CEF children can leave
that job so the shared browser process remains available to sibling windows.

## Paint and composition

Sabine uses one paint policy per platform. Apps cannot select a renderer or opt into experimental
transport branches. The GPU instance is shared within each process across window recreation.
Hibernation still releases per-window devices, surfaces, and textures; resuming reuses the
backend connection instead of repeatedly initializing graphics drivers.

- **Windows** uses accelerated `OnAcceleratedPaint`. CEF owns and pools the callback texture, so the
  CEF host first opens it on D3D11 and copies it into one of four Sabine-owned D3D12 shared textures
  before returning. Each destination is opened on the producer's D3D11 device for the copy and on
  wgpu's D3D12 device for composition. The host waits for its GPU copy before publishing the frame;
  the compositor samples the imported texture directly and sends a release acknowledgement only
  after submitted GPU work stops using it. The producer
  never reuses a slot before that acknowledgement. Physical texture dimensions remain separate
  from the visible source rectangle and logical window dimensions at non-integer display scales.
  Mailbox saturation drops an intermediate GPU frame and requests the newest paint; it never falls
  back to a synchronous CPU readback.
  Slots are isolated by browser and released when that browser closes. The compositor evicts
  retired guest and popup textures and clears page textures during hibernation.
- **Linux** uses CEF software `OnPaint` on Wayland and X11, with GPU composition in the native host.
  Launch selects the available display connection without rewriting the session type.
- **macOS** currently uses software `OnPaint`. An IOSurface path must copy or retain CEF's pooled
  resource before the callback returns; passing an IOSurface ID asynchronously is not sufficient.

Windows uses wgpu D3D12 rather than forcing Vulkan. Transparent windows use a DirectComposition
visual without an HWND redirection bitmap, allowing premultiplied OSR pixels to reveal the native
backdrop. Sabine applies Acrylic, blur, Mica, and Mica Alt directly through Win32 composition APIs.
On macOS, Sabine installs its own semantic `NSVisualEffectView` beneath the Metal content view.

CEF delivers BGRA dirty rectangles on the software path. Inline and shared-memory batches retain one
immutable byte backing instead of copying every rectangle into a separate allocation. The native
host patches one backing store per surface and uploads only the changed ranges to an independent GPU
texture (sparse per-rect uploads when damage is disjoint):

- main page
- popup overlay
- one texture per guest (including guest `<select>` popups)

The display list damages the union of the old and new bounds when a primitive changes. GPU vertex
buffers grow geometrically and are reused across redraws. Native surface resize is presented
synchronously with the last frame at its original logical size while CEF catches up. Resize control
messages coalesce to the newest size on CEF's UI thread. Main paints must exactly match that logical
size, and accelerated paints with transitional coded/content/source metadata are discarded. This
keeps the swapchain responsive without stretching a stale frame or presenting a partially relaid-out
Chromium texture.

Automatic frame pacing follows the monitor's current display mode and updates when the window
moves, changes scale or size, or regains focus. An explicit lifecycle frame rate overrides this
selection. Launch metrics start before configuration validation and include shared-runtime setup
and runtime resolution. Window `maximize()` is idempotent; `toggleMaximize()` toggles maximization,
and `restore()` leaves fullscreen, maximization, and minimization.

On Windows and macOS, `SabineProcess::wait()` runs the parent process's native event loop. Tray
icons and global shortcuts are created on that thread after the loop starts; process exits and
window commands wake it without polling. Use `SabineWindow::main` or call `wait()` on the main
thread when managing the process yourself. Update confirmation runs in a separate native prompt
process so it cannot consume the application's event loop before launch.

Transport is platform-specific without changing the protocol. On Unix, sockets live under
`$XDG_RUNTIME_DIR/sabine/<app_id>/` (mode `0700`) with socket mode `0600`, and each window
authenticates with a first-line token read from a one-use `0600` token file. The environment is
only a fallback for launches that do not need Chromium process-singleton handoff.

Paint messages use the versioned `SAB1` wire signature. Surface dimensions, inline payloads, and
shared mappings are bounded before allocation or mapping. The native host's paint queue limits
both message count and retained payload memory to 256 MiB. Merging preserves dirty rectangles while
limiting each merged batch to 256 rectangles and 64 MiB of retained buffers; larger batches remain
ordered and apply backpressure without dropping damage.
Control queues are limited to 256 messages and 64 MiB on each side. Pointer motion and
consecutive state updates coalesce; an ordered-command overflow closes the connection instead
of silently losing a command. Socket writes have a five-second deadline. CEF applies
backpressure before posting UI tasks and rejects unterminated control lines at 64 MiB.

| Platform | Transport | Paint path |
| --- | --- | --- |
| Linux | Unix domain socket | dirty-rect BGRA `OnPaint` → sparse wgpu uploads |
| macOS | Unix domain socket | dirty-rect BGRA `OnPaint` → sparse wgpu uploads |
| Windows | localhost TCP | CEF D3D11 → acknowledged NT texture slots → wgpu D3D12 |

Linux layer-shell surfaces use their dedicated host because layer-shell configuration must happen
before a regular winit surface is created. Other platforms express palette behavior with a normal
frameless, always-on-top, hide-on-blur window.

Initially hidden and prewarmed layer surfaces stay detached. With `retain_hidden_frame`, showing a
surface immediately restores size, anchors, margin, layer, exclusive zone, keyboard mode, alpha, and
background effect, then attaches the retained released SHM frame without waiting for another CEF
paint. A busy presentation buffer schedules an immediate retry and wakes again on `wl_buffer.release`.
`set_shell_surface_visible` queues the change and returns a `ShellSurfaceVisibilityRequest`
immediately. Poll its state for the asynchronous compositor-facing `Mapped` or `Unmapped`
acknowledgement. A newer request completes any superseded request with the surface's actual state
before applying the new target, so rapid toggles never block the shell thread or leave an
acknowledgement pending forever. Shells that change visibility, alpha, and margin together use
`set_shell_surface_presentation`; the layer host applies all three before the compositor-facing
commit, so a remap cannot acknowledge an intermediate hidden geometry. Layer loading uses the
compact three-line native animation without text so small shell surfaces do not reserve message
space.

Layer surfaces also accept live size and frame-rate changes through their existing host connection.
The host reconfigures the current Wayland surface and CEF browser in place, so responsive shell
content and output refresh changes do not restart or discard a prewarmed browser process.

## Runtime ownership

`sabine-runtime` is the only crate allowed to decide runtime locations, versions, download archives,
integrity, install locks, and pruning. The runtime is always CEF. WebView2 is not an alternate backend or a
fallback.

An install has these phases:

```text
plan -> lock -> download -> SHA-1 verify -> extract -> atomic version install -> ready
```

The Spotify CEF index supplies archive metadata and checksums. Runtime versions are immutable
directories, allowing existing apps to finish on an older version while the service installs a newer
one. Runtime and CEF-host builds have independent locks. Downloads use CEF's Minimal distribution,
which retains the build SDK while omitting Debug libraries and sample applications. Consumer runtime
detection requires the platform CEF binary, `icudtl.dat`, `resources.pak`, and at least one locale pack;
headers and CMake files are required only when compiling the host.

Interrupted CEF downloads retain their bytes and resume with validated HTTP byte ranges. Servers
that do not support ranges restart the transfer. Archive sizes are checked against the index before
SHA-1 verification; transient network/server failures allow up to four attempts, with a 30-minute
transfer deadline. Closing the setup window cancels its process and preserves the partial download.
Setup failures remain visible until dismissed, then exit without opening a second error window.

Runtime extraction runs in process and rejects unsafe paths, links leaving the version directory,
and special device entries. Extracted assets are validated before replacing an incomplete runtime,
and a runtime leased by a running app cannot be repaired in place. Offline bundles contain only
release binaries, resources, version metadata, and licensing files, preserving framework symlinks.
Build offline bundles on their target OS so the embedded runtime and service match the application.
Shared-system and managed-app ZIP and tar.gz extraction also runs in process, with confined paths,
validated links, and limits on entry count and expanded size. Archive type comes from its contents,
so download URLs may contain query parameters. ZIP extraction preserves executable permissions
and internal framework symlinks.

`sabine-service` owns the machine/user-level catalog. Its registry writes use a temporary file,
`sync_all`, and atomic rename. Re-registering an app preserves its original registration timestamp.
The dedicated `sabine-service-daemon` owns its PID file and maintenance loop. Linux starts it with a
user systemd unit, macOS with a LaunchAgent, and Windows with a hidden per-user scheduled task; the
Windows binary uses the GUI subsystem and never creates a console host. The maintenance loop updates
CEF to the newest compatible archive and keeps two runtime versions. Runtime failures
are reported independently from app updates. Incompatible applications remain registered
so their update source can deliver a compatible build. Maintenance reports retain each
failure and identify when no usable runtime is available. Linux runtime storage honors
`XDG_DATA_HOME`, using the same base directory as the service.
The user systemd unit follows `XDG_CONFIG_HOME` and preserves custom data, config, and cache paths.
Executable paths containing spaces or systemd specifier characters are quoted.

Browser profiles live under the native per-user cache location: `LOCALAPPDATA` on Windows,
`~/Library/Caches` on macOS, and `XDG_CACHE_HOME` on Linux. Sabine lets Chromium select the platform
credential store. Remote DevTools remains opt-in and uses Chromium's origin restrictions.

## Public API

The primary API is a fluent `SabineWindow` builder. Configuration is grouped by concern even though
the builder keeps common cases one method away:

- content: local entry, production URL, or a dev URL already owned by the CLI
- window: size, chrome, visibility, natural background color, transparency, blur and control regions
- browser: Chromium flags, devtools, profiles and security
- bridge: descriptors, sync handlers and async handlers
- lifecycle: foreground/background rates, suspend, hibernate and memory-saver policy
- services: tray, autostart, shortcuts, deep links, native messaging and single instance
- runtime: minimum version and bundled/shared policy

The API deliberately avoids backend types. Apps do not select paint transports or Chromium launch
flags. The primary surface is `SabineWindow`, its recipes, typed background effects and regions,
bridge handlers, lifecycle policy, desktop integrations, and `RuntimeConfig`. Runtime maintenance,
service policy, and native-host build helpers stay in their owning crates instead of being re-exported
through `sabine`.

## Bridge security

Bridge commands must be registered before launch. Each command can constrain targets and origins.
The host rejects unknown commands, invalid targets, and origins outside the configured allowlist.

Only the configured local entry document is trusted automatically; its query and fragment may change.
Other local files, opaque documents, and developer tools pages receive no implicit bridge access.
Remote documents must have a matching document security origin, so a CSP sandbox cannot inherit
privileges merely by retaining an allowed URL.
Calling `.url(...)` adds that URL's exact origin; development URLs add loopback variants needed by
local toolchains.

Native window and guest operations pass through the same document checks before dispatch.
A command-specific origin grants that command only, without granting window controls or app events.
Responses are bound to their originating V8 context and native request identity; navigation,
cancellation, or caller request-ID reuse cannot redirect a response into another document.
Resolved and failed JavaScript calls release their timeout immediately.

Guests default to `allow_bridge = false`. Explicitly supplied HTML in an opted-in guest remains
trusted when the app replaces it through `guest.navigate`; arbitrary data URLs do not gain access. A guest gets an isolated request context when it declares a
partition. Popup policy, download policy, visibility, bounds, intercepted shortcuts, and horizontal
wheel interception are all explicit guest properties.

While a guest is focused, matching `interceptedShortcuts` are consumed by the host: keydown emits
`guest.shortcut` to the primary page and neither keydown nor keyup reaches the guest. With
`interceptHorizontalWheel`, predominantly horizontal wheel samples emit `guest.wheel` and are not
forwarded to the guest. Favicon URL changes emit `guest.favicon`.

Keyboard and IME text in the control protocol uses lossless percent decoding, including carriage returns, tabs, and Unicode. URL display decoding is reserved for URLs; it deliberately preserves some escaped control characters and cannot decode keyboard text.

Editable primary and guest content drives the platform input method on demand. CEF reports the
focused editor's input mode and composition caret bounds; the native host maps those to the system
IME, positions its candidate window, and forwards preedit selection and committed text back to the
focused browser. The normal desktop host also supplies bounded surrounding text and applies the
platform's requested surrounding-text deletions to inputs, text areas, and content-editable regions.
Ordinary page focus does not keep the input method enabled. Layer-shell surfaces currently support
IME activation, caret placement, preedit, and commit; surrounding-text deletion remains unavailable
until the layer-shell event-loop dependency exposes that part of text-input-v3.

Touch and tablet input is forwarded as CEF touch input instead of being collapsed into mouse events,
preserving pointer identity, pressure, and touch, pen, or eraser type. CEF's touch event API does not
carry tablet tilt.

HTML file drags from the primary page are promoted to native OS drags. Incoming URI-list drags,
including self-drops, are accepted by the native host and emitted to the primary page as
`window.fileDrag`. Each event carries its phase, absolute file paths, content coordinates, the
negotiated copy/move/link action, and whether it originated from the same Sabine window. Apps use
those coordinates to resolve their own semantic drop targets without exposing renderer internals to
the native host. This native drag path applies to regular desktop windows. The current layer-shell
event-loop dependency does not expose Wayland data-device drag-and-drop, so layer surfaces reject an
outgoing file drag immediately instead of leaving CEF's drag source pending.

CEF's default file chooser remains in place so file inputs use the operating system picker. Browser
context-menu items are removed; development launches add only an `Inspect element` command, while
production launches have no browser-style context menu. HTML title tooltips are presented by the
native compositor after a short delay in both regular and layer-shell hosts.

The web bridge exposes:

```text
window.sabine.bridge
window.sabine.window
window.sabine.lifecycle
window.sabine.activity
window.sabine.guest
```

## Lifecycle and performance

Sabine distinguishes active, background, suspended, hibernating, and hibernated states. Activity
leases allow durable Rust work or page work to block hibernation while it is genuinely active.
Blur and occlusion suspend only lower the windowless frame rate; they do not call CEF `WasHidden`,
so brief focus loss during interactive move does not blank the surface. Hibernation is what actually
hides the view and tears down its browser. The shared CEF process stays alive when it still owns
other windows. Waking creates a fresh browser through the same profile-singleton handoff path, and
connection generations prevent a late disconnect from the old browser from clearing the new one.

Visible windows remain unmapped until the first browser frame when startup or wake completes
quickly. After 120 milliseconds, Sabine presents a neutral native loading surface using the same
GPU compositor, window chrome, and app background configured with
`.background_color(SabineColor::rgb8(r, g, b))`. The default is `#111113`. The same color seeds CEF
before the document paints, avoiding a surface-color jump when the native loader disappears. Its
loading copy rotates every 3.2 seconds from a 70% practical, 20% whimsical, and 10% strange pool
without repeating consecutively. Layer-shell surfaces use the same delay, app background, copy,
and scheduled animation while keeping the browser backing buffer separate from the transient loader.
Live transport loss uses the wake path so a window recovers
instead of remaining frozen.

Memory saver is explicit. `SabineLifecyclePolicy::memory_saver_hidden_window()` or
`.memory_saver(true)` enables the aggressive hidden-window policy and prevents Chromium from
keeping a spare renderer warm. Normal browser-tab and hidden-window policies retain Chromium's
spare renderer so future navigations do not pay an avoidable process-start penalty.

Rules for renderer changes:

- never replace damage tracking with unconditional full-frame uploads
- retain the last frame only when the selected lifecycle policy asks for it
- keep hidden palette windows warm unless memory-saver policy opts into hibernation
- do not poll when a native event or deadline can wake the event loop
- keep bridge and paint traffic off the UI thread except for final state application

## Desktop services

Desktop service configuration belongs to the window so an app has one declarative startup surface.
Implementations must degrade by reporting unsupported capabilities, not by silently changing the app
model.

Current primitives cover tray menus, autostart, global shortcuts, deep links, native messaging,
single-instance activation, hidden windows, always-on-top windows, and palette behavior. Native
platform registration belongs in `sabine-platform` or `sabine-service`; CEF code must not own it.

## Bundles and installs

The CLI reads `Sabine.toml`, builds web assets and the selected Rust package, writes a normalized
production manifest into the platform resource layout, stages the remaining runtime files and
metadata, and invokes a local package tool when available. Apps do not locate or copy manifests.
Bundle versions use SemVer and inherit `package.version` from Cargo, including workspace values,
when `[app].version` is absent. App IDs contain lowercase letters, digits, dots and hyphens.
The configured web output and entry must exist before staging; `--no-web-build` packages an
existing build. The output directory must not overlap the web assets or contain the source project.
Linux launchers forward file and URL arguments. Generated package scripts support paths containing
spaces and apostrophes. The `linux` target produces a filesystem tarball; Flatpak packaging is not
implemented.

`sabine dev` passes the source manifest and development URL directly to the runtime; `dev_url` and
`dev_port` are therefore never baked into production launch behavior.

Supported targets are portable, Linux directory, deb, rpm, AppImage, Windows directory, exe, msi,
macOS app, and dmg. Cross-host staging is allowed; signing and notarization remain deployment policy.
Linux packages keep each application, its resources, and any offline runtime/service payload under
`/usr/lib/sabine/<app-id>`. A relative symlink in `/usr/bin` launches the app. Runtime and manifest
discovery follow the executable into its private directory, including in relocated portable bundles
and AppImages. Separate applications do not claim shared offline helper binaries or runtime files.
Debian and RPM packages derive their architecture from the app's ELF binary. Build Debian packages
on the target Debian/Ubuntu environment with `dpkg-dev`: `dpkg-shlibdeps` determines linked-library
versions, supplemented by Chromium's desktop dependencies. RPM also declares the desktop libraries
needed by downloaded runtimes and retains automatic ELF dependency generation.

`[app]` accepts `publisher`, `maintainer`, and `license`. Publisher and maintainer can come from Cargo
`authors`, including inherited workspace authors; license can come from Cargo `license`. Debian
packages require a maintainer in `Name <email>` form. RPM packages require a license identifier.
These values describe your application; Sabine does not insert its own publisher or a placeholder
email address. Prerelease SemVer versions use `~` in Debian/RPM metadata so they sort before the
corresponding stable version.

Windows builds statically link the Microsoft C runtime and select the GUI subsystem. MSI output is
explicitly x64 and installs under the current user profile. Its wizard prepares the shared runtime,
supports repair, and rolls back app registration alongside packaged files on failure. Uninstall
removes the app registration and preserves the shared Sabine system. MSI builds require WiX 7 with
the UI and Util extensions. The configured icon appears in the Start menu shortcut, and the release
workflow installs and launches the package before publication. Raster icons are encoded directly; SVG
packaging requires ImageMagick so the CLI can generate Windows and freedesktop raster icon sizes.
The released Sabine CLI, service, and daemon also use the static Microsoft runtime. First-launch
downloads use the in-process HTTP client, and unavoidable helper processes are created without a
console so only the native bootstrap progress window is visible.

On macOS, the host and its helpers load the selected shared Chromium framework by its explicit
path. Starting an app does not write a framework symlink into the host bundle, and the host package
does not duplicate the CEF framework. Native CI signs the host and helpers, makes the bundle read-only,
initializes Chromium, and verifies that the signature remains intact.

Source installs are development conveniences. They stage assets and a launcher under the Sabine data
directory, register the app with the service, and create platform launch metadata.

## Validation

The manual `Native CEF host checks` workflow builds the C++ host against the current Minimal CEF
SDK on Linux, Windows, and Apple Silicon macOS, then runs `runtime doctor`. It does
not publish a release. This checks native compilation and Chromium initialization; desktop rendering,
input, multi-window behavior, and GPU/device acceptance require the platform runtime checks too.
The runtime probe shuts Chromium down normally and captures bounded error details without blocking
on stderr output.

After code changes run:

```sh
cargo fmt
cargo build --workspace
cargo test --workspace
cargo check --target x86_64-pc-windows-gnu --workspace
```

CI enforces formatting, warning-free Clippy, and workspace tests on Linux, then runs the workspace
tests natively on Windows MSVC and macOS as well.

The Windows cross-check validates Rust cfg coverage. A release still needs a native Windows CEF-host
build and an actual GPU/input smoke test. The same rule applies to macOS. Linux tests do not prove
Windows or macOS composition behavior.

On Windows, `sabine-host` must be built with **MSVC** (Visual Studio 2019+ C++ workload). Official CEF
binaries do not link with MinGW/MSYS. Sabine forces a Visual Studio CMake generator and normalizes
`CEF_ROOT` to forward slashes so CEF’s cmake macros do not treat `\Users\...` as escape sequences.

CEF archives are extracted with `tar` using the destination as the process working directory (not
`tar -C C:\...`). Git for Windows’ GNU tar treats a drive letter in `-C` as a remote host and can
leave a partial tree that looks installed but cannot build the host.

### Startup diagnostics

Startup, setup, OSR host, Chromium host, and maintenance errors are saved as JSON lines
in the shared Sabine data directory under `logs/`. Each component keeps up to 4 MiB
before discarding older entries. On Linux this directory follows `XDG_DATA_HOME`,
falling back to `~/.local/share/sabine/logs`. Child stderr is also forwarded to the
launching terminal when one is attached. Diagnostic files may contain application
URLs and paths; review their contents before sharing them.

Failed setup keeps its progress window open, displays the error and log location,
and closes with Enter, Escape, or the window close button. Startup failures before
the app window opens display a separate notice. The windows render Unicode text
using installed fonts and wrap messages; the log retains text beyond the visible
window area.

The native host and app must support the same host protocol. Sabine checks the host before
launching Chromium and reports a repair/update error when an older installation cannot enforce
the app's bridge policy.

Packaged apps use an installed host and never invoke a compiler on the user's machine. Development
builds and the packaging CLI can build a host from the CEF SDK. Compatible newer shared hosts are
selected by their native protocol rather than requiring an identical app framework version.

### Windows setup wizard

The `exe` bundle target generates an NSIS wizard. The installer runs the app with
`--sabine-install` after copying its files. This prepares the service and Chromium runtime,
checks native host compatibility, and registers the app without launching its window. Progress
and failures go to the installer details pane; failure returns a nonzero exit code.
`--sabine-uninstall` removes the registration before the uninstaller deletes packaged files.
These modes are handled by `SabineWindow::main` and its process variants.

The default destination is `%LOCALAPPDATA%\Programs\<app-id>`. Installation and shortcuts belong
to the current user. The uninstaller deletes the files included by the package and leaves other
files in place. Shared runtimes and the service remain available to other Sabine apps.
Offline bundles supply the same setup path with embedded dependencies.
