#import <Cocoa/Cocoa.h>

#include <string>
#include <dlfcn.h>
#include <iostream>
#include "sabine_host_protocol.h"

#include "entry.h"
#include "include/cef_application_mac.h"
#include "include/cef_sandbox_mac.h"
#include "include/wrapper/cef_helpers.h"
#include "include/wrapper/cef_library_loader.h"

@interface SabineApplication : NSApplication <CefAppProtocol> {
 @private
  BOOL handlingSendEvent_;
}
@end

@implementation SabineApplication
- (BOOL)isHandlingSendEvent {
  return handlingSendEvent_;
}

- (void)setHandlingSendEvent:(BOOL)handlingSendEvent {
  handlingSendEvent_ = handlingSendEvent;
}

- (void)sendEvent:(NSEvent*)event {
  CefScopedSendingEvent sendingEvent;
  [super sendEvent:event];
}
@end

namespace {
class SandboxLibrary {
 public:
  ~SandboxLibrary() {
    if (context_) destroy_(context_);
    if (library_) dlclose(library_);
  }

  bool Initialize(const std::string& framework, int argc, char* argv[]) {
    const auto path = framework + "/Libraries/libcef_sandbox.dylib";
    library_ = dlopen(path.c_str(), RTLD_LAZY | RTLD_LOCAL | RTLD_FIRST);
    if (!library_) {
      std::cerr << "Could not load the Chromium sandbox: " << dlerror() << std::endl;
      return false;
    }
    const auto initialize = reinterpret_cast<decltype(&cef_sandbox_initialize)>(
        dlsym(library_, "cef_sandbox_initialize"));
    destroy_ = reinterpret_cast<decltype(&cef_sandbox_destroy)>(
        dlsym(library_, "cef_sandbox_destroy"));
    if (!initialize || !destroy_) {
      std::cerr << "Chromium sandbox entry points are missing" << std::endl;
      return false;
    }
    context_ = initialize(argc, argv);
    if (!context_) {
      std::cerr << "Chromium helper sandbox initialization failed" << std::endl;
    }
    return context_ != nullptr;
  }

 private:
  void* library_ = nullptr;
  void* context_ = nullptr;
  decltype(&cef_sandbox_destroy) destroy_ = nullptr;
};

class FrameworkLibrary {
 public:
  explicit FrameworkLibrary(const std::string& directory)
      : loaded_(!directory.empty() && cef_load_library(
          (directory + "/Chromium Embedded Framework").c_str())) {}
  ~FrameworkLibrary() { if (loaded_) cef_unload_library(); }
  bool loaded() const { return loaded_; }
 private:
  const bool loaded_;
};
}

int main(int argc, char* argv[]) {
  bool subprocess = false;
  std::string framework;
  for (int index = 1; index < argc; ++index) {
    const std::string argument(argv[index]);
    if (argument == "--sabine-host-protocol") {
      std::cout << SABINE_HOST_PROTOCOL_VERSION << std::endl;
      return 0;
    }
    if (argument.rfind("--type=", 0) == 0 || argument == "--type") {
      subprocess = true;
    }
    const std::string prefix = "--sabine-framework-dir-path=";
    if (argument.rfind(prefix, 0) == 0) {
      framework = argument.substr(prefix.size());
    }
  }

  SandboxLibrary sandbox;
  if (subprocess && !sandbox.Initialize(framework, argc, argv)) {
    return 1;
  }
  FrameworkLibrary library(framework);
  if (!library.loaded()) {
    std::cerr << "Could not load the Chromium framework at " << framework << std::endl;
    return 1;
  }

  @autoreleasepool {
    if (!subprocess) {
      [SabineApplication sharedApplication];
      CHECK([NSApp isKindOfClass:[SabineApplication class]]);
    }
    CefMainArgs main_args(argc, argv);
    return RunSabineHost(main_args, argc, argv);
  }
}
