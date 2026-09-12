#include "runtime/probe.h"

#include <cstdint>
#include <iostream>

#include "include/cef_app.h"
#include "include/base/cef_callback.h"
#include "include/cef_browser.h"
#include "include/cef_client.h"
#include "include/cef_render_handler.h"
#include "include/wrapper/cef_closure_task.h"

namespace {
int result = 1;

class RuntimeProbe : public CefClient,
                     public CefRenderHandler,
                     public CefLifeSpanHandler {
 public:
  CefRefPtr<CefRenderHandler> GetRenderHandler() override { return this; }
  CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override { return this; }

  void GetViewRect(CefRefPtr<CefBrowser>, CefRect& rect) override {
    rect = CefRect(0, 0, 64, 64);
  }

  void OnAfterCreated(CefRefPtr<CefBrowser> browser) override {
    browser_ = browser;
    CefRefPtr<RuntimeProbe> self(this);
    CefPostDelayedTask(TID_UI, CefCreateClosureTask(base::BindOnce(&RuntimeProbe::Timeout, self)), 25000);
  }

  void OnPaint(CefRefPtr<CefBrowser> browser, PaintElementType type,
               const RectList&, const void* buffer, int width, int height) override {
    if (type != PET_VIEW || width != 64 || height != 64 || closing_) return;
    const auto* pixels = static_cast<const uint32_t*>(buffer);
    if (pixels[0] != 0xff336699 || pixels[32 * width + 32] != 0xff336699) return;
    result = 0;
    closing_ = true;
    browser->GetHost()->CloseBrowser(true);
  }

  void OnBeforeClose(CefRefPtr<CefBrowser>) override {
    browser_ = nullptr;
    CefQuitMessageLoop();
  }

 private:
  void Timeout() {
    if (!browser_ || closing_) return;
    std::cerr << "Chromium did not render its runtime probe page within 25 seconds" << std::endl;
    closing_ = true;
    browser_->GetHost()->CloseBrowser(true);
  }

  CefRefPtr<CefBrowser> browser_;
  bool closing_ = false;
  IMPLEMENT_REFCOUNTING(RuntimeProbe);
};
}

void StartRuntimeProbe() {
  CefWindowInfo window;
  window.SetAsWindowless(0);
  CefBrowserSettings settings;
  settings.windowless_frame_rate = 60;
  if (!CefBrowserHost::CreateBrowser(window, new RuntimeProbe(),
      "data:text/html,<html style='background:%23336699'></html>", settings,
      nullptr, nullptr)) {
    std::cerr << "Chromium could not create an off-screen browser" << std::endl;
    CefQuitMessageLoop();
  }
}

int RuntimeProbeResult() { return result; }
