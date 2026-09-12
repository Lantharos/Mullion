#import <Cocoa/Cocoa.h>

#include <string>
#include <iostream>
#include "sabine_host_protocol.h"

#include "entry.h"
#include "include/cef_application_mac.h"
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
