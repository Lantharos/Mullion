#include "osr/handler.h"

#include <algorithm>
#include <cctype>
#include <cerrno>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <limits>
#include <set>
#include <sstream>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#else
#include <sys/socket.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/un.h>
#include <sys/uio.h>
#include <unistd.h>
#endif

#include "guest/input.h"
#include "guest/manager.h"
#include "include/cef_app.h"
#include "include/cef_browser.h"
#include "include/cef_parser.h"
#include "include/cef_request_context_handler.h"
#include "include/cef_task.h"
#include "include/internal/cef_types.h"
#include "include/wrapper/cef_helpers.h"
#include "common/json.h"
#include "common/bridge_policy.h"
#include "sabine_bridge_js.h"
#include "osr/utilities.h"

using namespace sabine_osr;

bool SabineOsrHandler::OnProcessMessageReceived(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefProcessId source_process,
    CefRefPtr<CefProcessMessage> message) {
  CEF_REQUIRE_UI_THREAD();
  if (source_process != PID_RENDERER || !message || !browser || !frame || !frame->IsValid() ||
      !BridgePolicyFor(browser)) {
    return false;
  }
  CefRefPtr<CefListValue> arguments = message->GetArgumentList();
  if (!arguments || arguments->GetSize() < 1 ||
      arguments->GetType(0) != VTYPE_STRING) {
    return true;
  }
  const std::string payload = arguments->GetString(0);
  if (message->GetName() == "sabine.ime_state") {
    const int browser_id = browser->GetIdentifier();
    ime_surrounding_state_[browser_id] = payload;
    ime_frames_[browser_id] = frame;
    SendFocusedImeState();
    return true;
  }
  if (message->GetName() != "sabine.native") {
    return false;
  }
  if (!frame->IsMain() || arguments->GetSize() != 2 || arguments->GetType(1) != VTYPE_STRING) return true;
  const auto policy = BridgePolicyFor(browser);
  const std::string url = frame->GetURL();
  if (!sabine_bridge::MatchesSecurityOrigin(policy, url, arguments->GetString(1))) return true;
  if (payload.rfind("sabine://window/", 0) == 0) {
    if (sabine_bridge::AllowsDocument(policy, url)) HandleWindowCommand(browser, payload);
    return true;
  }
  if (!sabine_bridge::AllowsCommand(policy, url, QueryValue(payload, "name"))) return true;
  return HandleBridgeCommand(browser, frame, payload);
}

bool SabineOsrHandler::HandleWindowCommand(CefRefPtr<CefBrowser> browser,
	                                         const std::string& url) {
  const std::string prefix = "sabine://window/";
  if (url.rfind(prefix, 0) != 0) {
    return false;
  }
  std::string command = url.substr(prefix.size());
  const size_t query = command.find_first_of("?#");
  if (query != std::string::npos) {
    command = command.substr(0, query);
  }
  if (command == "close") {
    // Ask the native host to tear down its window; it replies with "close\n"
    // which CloseBrowsers this surface. Do not quit the shared CEF process.
    RequestNativeClose();
  } else if (command == "start-drag" || command == "drag") {
    SendMessage(6, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "minimize") {
    SendMessage(7, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "maximize") {
    SendMessage(35, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "restore") {
    SendMessage(36, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "toggle-maximize") {
    SendMessage(8, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "fullscreen") {
    SendMessage(27, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "exit-fullscreen") {
    SendMessage(28, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "show") {
    SendMessage(9, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "hide") {
    SendMessage(10, 0, 0, 0, 0, nullptr, 0);
  } else if (command == "focus") {
    const std::string activation_token = QueryValue(url, "activationToken");
    SendMessage(11, 0, 0, 0, 0, activation_token.data(),
                static_cast<uint32_t>(activation_token.size()));
  }
  return true;
}

bool SabineOsrHandler::HandleBridgeCommand(CefRefPtr<CefBrowser> browser,
                                         CefRefPtr<CefFrame> frame,
                                         const std::string& url) {
  const std::string prefix = "sabine://bridge/";
  if (url.rfind(prefix, 0) != 0) {
    return false;
  }
  const std::string request_id = BridgeRequestId(url);
  const std::string command = QueryValue(url, "name");
  const std::string payload = QueryValue(url, "payload");
  const std::string browser_id = std::to_string(browser->GetIdentifier());
  std::string origin = sabine_bridge::Origin(frame->GetURL());
  if (origin.empty()) origin = frame->GetURL();
  else if (origin.back() == '/') origin.pop_back();
  if (request_id.empty() || command.empty()) {
    ResolveBridgeResponse(browser_id, request_id, false,
                          "{\"message\":\"Malformed Sabine bridge request\"}");
    return true;
  }
  const GuestView* guest = GuestForBrowser(browser);
  if (guest && !guest->allow_bridge) {
    ResolveBridgeResponse(
        browser_id, request_id, false,
        "{\"message\":\"Sabine bridge is unavailable inside guest views\"}");
    return true;
  }
  if (guest) {
    if (IsGuestBridgeCommand(command) || command == "sabine.popup.open" ||
        command == "sabine.popup.close") {
      ResolveBridgeResponse(browser_id, request_id, false,
                            "{\"message\":\"Guest views cannot manage other "
                            "guest views\"}");
      return true;
    }
  } else if (HandleGuestBridgeCommand(command, payload, browser_id,
                                      request_id)) {
    return true;
  }
  if (bridge_commands_.find(command) == bridge_commands_.end()) {
    ResolveBridgeResponse(
        browser_id, request_id, false,
        "{\"message\":\"Sabine bridge command is not allowlisted\"}");
    return true;
  }
  const std::string request_line =
      "SABINE_BRIDGE_REQUEST\t" + browser_id + "\t" + request_id + "\t" +
      origin + "\t" + command + "\t" + (payload.empty() ? "{}" : payload);
  SendMessage(kBridgeRequest, 0, 0, 0, 0, request_line.data(),
              static_cast<uint32_t>(request_line.size()));
  return true;
}

void SabineOsrHandler::RequestNativeClose() {
  SendMessage(5, 0, 0, 0, 0, nullptr, 0);
}

void SabineOsrHandler::CloseFromNativeDisconnect() {
  CEF_REQUIRE_UI_THREAD();
  close_requested_ = true;
  if (browser_) {
    browser_->GetHost()->CloseBrowser(true);
  }
}

CefRefPtr<CefDictionaryValue> SabineOsrHandler::BridgePolicyFor(CefRefPtr<CefBrowser> browser) {
  if (!browser) return nullptr;
  if (browser_ && browser_->IsSame(browser)) return bridge_policy_;
  const GuestView* guest = GuestForBrowser(browser);
  return guest ? guest->bridge_policy : nullptr;
}

void SabineOsrHandler::InstallTransparentBackground(CefRefPtr<CefFrame> frame) {
  if (!transparent_background_) {
    return;
  }
  frame->ExecuteJavaScript(
      "(function(){"
      "if(document.documentElement){document.documentElement.style.background='transparent';}"
      "if(document.body){document.body.style.background='transparent';}"
      "if(!document.querySelector('style[data-sabine-transparent-background]')){"
      "const style=document.createElement('style');"
      "style.setAttribute('data-sabine-transparent-background','');"
      "style.textContent='html,body{background:transparent!important;}';"
      "document.head&&document.head.appendChild(style);"
      "}"
      "})();",
      frame->GetURL(), 0);
}

void SabineOsrHandler::ResolveBridgeResponse(const std::string& browser_id,
                                           const std::string& request_id,
                                           bool ok,
                                           const std::string& payload) {
  CEF_REQUIRE_UI_THREAD();
  const int expected_id = std::atoi(browser_id.c_str());
  CefRefPtr<CefBrowser> target;
  for (auto& browser : browsers_) {
    if (browser->GetIdentifier() == expected_id) {
      target = browser;
      break;
    }
  }
  if (!target || request_id.empty()) {
    return;
  }
  auto response = CefProcessMessage::Create("sabine.response");
  auto values = response->GetArgumentList();
  values->SetString(0, request_id);
  values->SetBool(1, ok);
  values->SetString(2, payload.empty() ? "null" : payload);
  target->GetMainFrame()->SendProcessMessage(PID_RENDERER, response);
}

void SabineOsrHandler::EmitBridgeEvent(const std::string& name_json,
                                         const std::string& payload) {
  CEF_REQUIRE_UI_THREAD();
  for (auto& browser : browsers_) {
    if (!sabine_bridge::AllowsDocument(BridgePolicyFor(browser), browser->GetMainFrame()->GetURL())) {
      continue;
    }
    auto event = CefProcessMessage::Create("sabine.event");
    event->GetArgumentList()->SetString(0, name_json);
    event->GetArgumentList()->SetString(1, payload.empty() ? "null" : payload);
    browser->GetMainFrame()->SendProcessMessage(PID_RENDERER, event);
  }
}

void SabineOsrHandler::EmitPrimaryEvent(const std::string& name,
                                        const std::string& payload) {
  CEF_REQUIRE_UI_THREAD();
  if (!browser_) {
    return;
  }
  CefRefPtr<CefFrame> frame = browser_->GetMainFrame();
  if (!frame || !sabine_bridge::AllowsDocument(BridgePolicyFor(browser_), frame->GetURL())) {
    return;
  }
  auto event = CefProcessMessage::Create("sabine.event");
  event->GetArgumentList()->SetString(0, JsString(name));
  event->GetArgumentList()->SetString(1, payload.empty() ? "null" : payload);
  frame->SendProcessMessage(PID_RENDERER, event);
}
