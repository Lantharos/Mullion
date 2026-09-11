#include "osr/handler.h"

#include <cstdio>

#include "common/json.h"
#include "include/wrapper/cef_helpers.h"

void SabineOsrHandler::OnRenderProcessTerminated(
    CefRefPtr<CefBrowser> browser, TerminationStatus status,
    int error_code, const CefString& error_string) {
  CEF_REQUIRE_UI_THREAD();
  if (closing_ || close_requested_) return;
  const auto now = std::chrono::steady_clock::now();
  auto& crashes = renderer_crashes_[browser->GetIdentifier()];
  while (!crashes.empty() && now - crashes.front() >= std::chrono::seconds(60)) {
    crashes.pop_front();
  }
  const bool recovering = crashes.size() < 3;
  crashes.push_back(now);
  const GuestView* guest = GuestForBrowser(browser);
  const std::string guest_id = guest ? guest->id : "";
  std::fprintf(stderr, "Sabine renderer terminated: browser=%d status=%d code=%d recovering=%d %s\n",
               browser->GetIdentifier(), static_cast<int>(status), error_code,
               recovering ? 1 : 0, error_string.ToString().c_str());
  EmitPrimaryEvent("runtime.renderer-crashed",
      "{\"guestId\":\"" + JsonEscape(guest_id) + "\",\"code\":" +
      std::to_string(error_code) + ",\"recovering\":" +
      (recovering ? "true" : "false") + "}");
  if (recovering) {
    if (!guest) SendMessage(kMainLoadStarted, 0, 0, 0, 0, nullptr, 0);
    browser->Reload();
    return;
  }
  if (!guest) {
    const std::string message = "The page renderer stopped repeatedly. Close the application and try again.";
    SendMessage(37, 0, 0, 0, 0, message.data(), static_cast<uint32_t>(message.size()));
  }
  browser->GetHost()->CloseBrowser(true);
}
