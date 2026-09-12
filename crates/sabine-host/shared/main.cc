#include "app/app.h"
#include "runtime/probe.h"

#if defined(OS_WIN) || defined(_WIN32)
#include <windows.h>
#include "include/cef_sandbox_win.h"
#include "include/cef_version_info.h"
#endif

#if defined(CEF_X11)
#include <X11/Xlib.h>
#endif

#include <cstdlib>
#include <iostream>
#include "sabine_host_protocol.h"
#include <string>

#include "include/base/cef_compiler_specific.h"
#include "include/cef_app.h"
#include "include/cef_command_line.h"
#include "entry.h"

#if defined(CEF_X11)
namespace {
int XErrorHandlerImpl(Display* display, XErrorEvent* event) {
  return 0;
}

int XIOErrorHandlerImpl(Display* display) {
  return 0;
}
}  // namespace
#endif

#if defined(OS_LINUX)
NO_STACK_PROTECTOR
#endif
int RunSabineHost(CefMainArgs main_args, int argc, char* argv[], void* sandbox_info) {
  CefRefPtr<CefCommandLine> command_line = CefCommandLine::CreateCommandLine();
#if defined(OS_WIN) || defined(_WIN32)
  command_line->InitFromString(::GetCommandLineW());
#else
  command_line->InitFromArgv(argc, argv);
#endif
  if (command_line->HasSwitch("sabine-host-protocol")) {
    std::cout << SABINE_HOST_PROTOCOL_VERSION << std::endl;
    return 0;
  }
  const bool runtime_smoke_test =
      command_line->HasSwitch("sabine-runtime-smoke-test");
  CefRefPtr<SabineApp> app(new SabineApp(runtime_smoke_test));

  int exit_code = CefExecuteProcess(main_args, app.get(), sandbox_info);
  if (exit_code >= 0) {
    return exit_code;
  }

#if defined(CEF_X11)
  XSetErrorHandler(XErrorHandlerImpl);
  XSetIOErrorHandler(XIOErrorHandlerImpl);
#endif

  CefSettings settings;
#if defined(OS_LINUX)
  settings.no_sandbox = true;
#endif
  settings.windowless_rendering_enabled = true;

  const std::string resources_dir_path =
      command_line->GetSwitchValue("sabine-resources-dir-path");
  if (!resources_dir_path.empty()) {
    CefString(&settings.resources_dir_path).FromString(resources_dir_path);
  }

  const std::string locales_dir_path =
      command_line->GetSwitchValue("sabine-locales-dir-path");
  if (!locales_dir_path.empty()) {
    CefString(&settings.locales_dir_path).FromString(locales_dir_path);
  }

  const std::string root_cache_path =
      command_line->GetSwitchValue("root-cache-path");
  if (!root_cache_path.empty()) {
    CefString(&settings.root_cache_path).FromString(root_cache_path);
  }

  const std::string cache_path = command_line->GetSwitchValue("cache-path");
  if (!cache_path.empty()) {
    CefString(&settings.cache_path).FromString(cache_path);
  }

  if (!CefInitialize(main_args, settings, app.get(), sandbox_info)) {
    return CefGetExitCode();
  }

  CefRunMessageLoop();
  const int result = runtime_smoke_test ? RuntimeProbeResult() : 0;
  CefShutdown();
  return result;
}

#if defined(OS_WIN) || defined(_WIN32)
CEF_BOOTSTRAP_EXPORT int RunWinMain(HINSTANCE instance,
                                    LPWSTR command_line,
                                    int show,
                                    void* sandbox_info,
                                    cef_version_info_t* version_info) {
  (void)command_line;
  (void)show;
  (void)version_info;
  if (!sandbox_info) {
    std::cerr << "Sabine requires the Chromium sandbox bootstrap" << std::endl;
    return 1;
  }
  CefMainArgs main_args(instance);
  return RunSabineHost(main_args, __argc, __argv, sandbox_info);
}
#elif !defined(OS_MAC)
NO_STACK_PROTECTOR
int main(int argc, char* argv[]) {
  CefMainArgs main_args(argc, argv);
  return RunSabineHost(main_args, argc, argv);
}
#endif
