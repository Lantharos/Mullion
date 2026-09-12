#include "osr/handler.h"

#include "include/wrapper/cef_helpers.h"

bool SabineOsrHandler::OnShowPermissionPrompt(
    CefRefPtr<CefBrowser>,
    uint64_t,
    const CefString&,
    uint32_t,
    CefRefPtr<CefPermissionPromptCallback> callback) {
  CEF_REQUIRE_UI_THREAD();
  callback->Continue(CEF_PERMISSION_RESULT_DENY);
  return true;
}
