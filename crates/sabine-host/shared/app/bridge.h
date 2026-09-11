#ifndef SABINE_RENDERER_BRIDGE_H_
#define SABINE_RENDERER_BRIDGE_H_
#include "include/cef_browser.h"
#include "include/cef_v8.h"
namespace sabine_bridge {
void InstallTransport(CefRefPtr<CefFrame> frame, CefRefPtr<CefV8Context> context,
                      const char* name, const char* message);
std::string RememberContext(CefRefPtr<CefV8Context> context);
void ReleaseContext(CefRefPtr<CefV8Context> context);
void ReleaseBrowser(CefRefPtr<CefBrowser> browser);
bool Receive(CefRefPtr<CefBrowser> browser, CefRefPtr<CefFrame> frame,
             CefRefPtr<CefProcessMessage> message,
             CefRefPtr<CefDictionaryValue> policy);
}
#endif
