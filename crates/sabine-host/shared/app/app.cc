#include "app/app.h"
#include "app/bridge.h"
#include "common/bridge_policy.h"

#include <sstream>
#include <algorithm>
#include <string>
#include <utility>
#include <vector>

#include "sabine_bridge_js.h"
#include "include/cef_browser.h"
#include "include/cef_command_line.h"
#include "include/cef_process_message.h"
#include "include/wrapper/cef_helpers.h"
#include "osr/handler.h"
#include "osr/utilities.h"

namespace {
const char kImeStateScript[] = R"JS(
(() => {
  if (window.__sabineImeInstalled) return;
  window.__sabineImeInstalled = true;
  let queued = false;
  const editable = () => {
    const element = document.activeElement;
    if (!element) return null;
    if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
      if (element instanceof HTMLInputElement && element.type === 'password') return null;
      if (typeof element.selectionStart !== 'number') return null;
      return { element, text: element.value, anchor: element.selectionStart,
               cursor: element.selectionEnd, control: true };
    }
    if (!element.isContentEditable) return null;
    const selection = getSelection();
    if (!selection || selection.rangeCount === 0 || !element.contains(selection.anchorNode) ||
        !element.contains(selection.focusNode)) return null;
    const offset = (node, position) => {
      const range = document.createRange();
      range.selectNodeContents(element);
      range.setEnd(node, position);
      return range.toString().length;
    };
    return { element, text: element.textContent || '',
             anchor: offset(selection.anchorNode, selection.anchorOffset),
             cursor: offset(selection.focusNode, selection.focusOffset), control: false };
  };
  const snapshot = () => {
    queued = false;
    const state = editable();
    if (!state) {
      __sabineImeState(JSON.stringify({ text: '', cursor: 0, anchor: 0, base: 0 }));
      return;
    }
    const low = Math.min(state.anchor, state.cursor);
    const high = Math.max(state.anchor, state.cursor);
    let start = Math.max(0, low - 1500);
    let end = Math.min(state.text.length, Math.max(high + 1500, start + 3000));
    if (start > 0 && /[\uDC00-\uDFFF]/.test(state.text[start])) start--;
    if (end < state.text.length && /[\uDC00-\uDFFF]/.test(state.text[end])) end--;
    __sabineImeState(JSON.stringify({ text: state.text.slice(start, end),
      cursor: state.cursor - start, anchor: state.anchor - start, base: start }));
  };
  const queue = () => {
    if (!queued) {
      queued = true;
      queueMicrotask(snapshot);
    }
  };
  const textPosition = (root, offset) => {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let total = 0;
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const length = node.nodeValue.length;
      if (offset <= total + length) return [node, offset - total];
      total += length;
    }
    return [root, root.childNodes.length];
  };
  window.__sabineImeDelete = (start, end) => {
    const state = editable();
    if (!state || start < 0 || end < start || end > state.text.length) return;
    if (state.control) {
      state.element.setSelectionRange(start, end);
    } else {
      const range = document.createRange();
      const from = textPosition(state.element, start);
      const to = textPosition(state.element, end);
      range.setStart(from[0], from[1]);
      range.setEnd(to[0], to[1]);
      const selection = getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
    }
    if (!document.execCommand('delete')) {
      if (state.control) {
        state.element.setRangeText('', start, end, 'end');
        state.element.dispatchEvent(new InputEvent('input', { bubbles: true,
          inputType: 'deleteContentBackward' }));
      } else {
        getSelection().getRangeAt(0).deleteContents();
        state.element.dispatchEvent(new InputEvent('input', { bubbles: true,
          inputType: 'deleteContentBackward' }));
      }
    }
    queue();
  };
  document.addEventListener('focusin', queue, true);
  document.addEventListener('focusout', queue, true);
  document.addEventListener('input', queue, true);
  document.addEventListener('selectionchange', queue, true);
  queue();
})();
)JS";

std::string JsString(const std::string& value) {
  std::string output = "\"";
  for (char character : value) {
    switch (character) {
      case '\\': output += "\\\\"; break;
      case '"': output += "\\\""; break;
      case '\n': output += "\\n"; break;
      case '\r': output += "\\r"; break;
      case '\t': output += "\\t"; break;
      default: output += character; break;
    }
  }
  return output + "\"";
}

std::string BridgeInstallScript(const std::vector<std::string>& commands) {
  std::string list = "[";
  for (size_t index = 0; index < commands.size(); ++index) {
    if (index > 0) {
      list += ",";
    }
    list += JsString(commands[index]);
  }
  return "window.__sabineBridgeCommands=" + list + "];" +
         SABINE_BRIDGE_JS_RAW;
}

void CreateBrowser(CefRefPtr<CefCommandLine> command_line) {
  if (!CreateSabineOsrBrowser(command_line)) {
    std::fprintf(stderr, "Sabine OSR: browser creation failed\n");
    if (!sabine_osr::HasRegisteredHandlers()) CefQuitMessageLoop();
  }
}
}  // namespace

SabineApp::SabineApp(bool runtime_smoke_test)
    : runtime_smoke_test_(runtime_smoke_test) {}

void SabineApp::OnBeforeCommandLineProcessing(
    const CefString& process_type,
    CefRefPtr<CefCommandLine> command_line) {
  const std::string ozone_platform =
      command_line->GetSwitchValue("sabine-ozone-platform");
#if defined(OS_LINUX)
  const bool disable_vulkan = true;
#else
  const bool disable_vulkan = false;
#endif
  if (disable_vulkan) {
    command_line->AppendSwitch("disable-vulkan");
  }
  if (!ozone_platform.empty()) {
    command_line->AppendSwitchWithValue("ozone-platform", ozone_platform);
    command_line->AppendSwitchWithValue("ozone-platform-hint", ozone_platform);
  }

  // Merge into any disable-features already set on the argv (do not replace).
  std::string disabled =
      command_line->GetSwitchValue("disable-features").ToString();
  auto ends_with_comma_or_empty = [&disabled]() {
    return disabled.empty() || disabled.back() == ',';
  };
  auto append_csv = [&disabled, &ends_with_comma_or_empty](const char* csv) {
    if (!ends_with_comma_or_empty()) {
      disabled += ",";
    }
    disabled += csv;
  };
  if (disabled.find("OptimizationGuideOnDeviceModel") == std::string::npos) {
    append_csv("OptimizationGuideOnDeviceModel");
  }
  if (disable_vulkan) {
    append_csv("Vulkan,DefaultANGLEVulkan,VulkanFromANGLE");
  }
  command_line->AppendSwitchWithValue("disable-features", disabled);

  if (command_line->HasSwitch("sabine-transparent")) {
    command_line->AppendSwitch("enable-transparent-visuals");
    command_line->AppendSwitch("transparent-painting-enabled");
    command_line->AppendSwitchWithValue("default-background-color", "0x00000000");
  }
}

void SabineApp::OnContextInitialized() {
  CEF_REQUIRE_UI_THREAD();
  if (runtime_smoke_test_) {
    return;
  }
  CreateBrowser(CefCommandLine::GetGlobalCommandLine());
}

void SabineApp::OnBrowserCreated(CefRefPtr<CefBrowser> browser,
                                  CefRefPtr<CefDictionaryValue> extra_info) {
  CEF_REQUIRE_RENDERER_THREAD();
  if (browser) bridge_policies_.push_back({browser, extra_info});
}

void SabineApp::OnBrowserDestroyed(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_RENDERER_THREAD();
  if (browser) {
    bridge_policies_.erase(std::remove_if(bridge_policies_.begin(), bridge_policies_.end(),
        [&](const BrowserPolicy& entry) { return entry.browser->IsSame(browser); }),
        bridge_policies_.end());
    sabine_bridge::ReleaseBrowser(browser);
  }
}

CefRefPtr<CefDictionaryValue> SabineApp::BridgePolicyFor(CefRefPtr<CefBrowser> browser) {
  for (const auto& entry : bridge_policies_) {
    if (entry.browser->IsSame(browser)) return entry.policy;
  }
  return nullptr;
}

void SabineApp::OnContextCreated(CefRefPtr<CefBrowser> browser,
                                  CefRefPtr<CefFrame> frame,
                                  CefRefPtr<CefV8Context> context) {
  CEF_REQUIRE_RENDERER_THREAD();
  sabine_bridge::InstallTransport(frame, context, "__sabineImeState", "sabine.ime_state");
  frame->ExecuteJavaScript(kImeStateScript, frame->GetURL(), 0);
  const auto policy = BridgePolicyFor(browser);
  const std::string security_origin = sabine_bridge::RememberContext(context);
  if (!frame->IsMain() || !sabine_bridge::ExposesBridge(policy, frame->GetURL()) ||
      !sabine_bridge::MatchesSecurityOrigin(policy, frame->GetURL(), security_origin)) return;
  sabine_bridge::InstallTransport(frame, context, "__sabineNativePostMessage", "sabine.native");
  const auto commands = sabine_bridge::Commands(policy);
  frame->ExecuteJavaScript(BridgeInstallScript(commands), frame->GetURL(), 0);
}

void SabineApp::OnContextReleased(CefRefPtr<CefBrowser> browser,
                                  CefRefPtr<CefFrame> frame,
                                  CefRefPtr<CefV8Context> context) {
  CEF_REQUIRE_RENDERER_THREAD();
  sabine_bridge::ReleaseContext(context);
}

bool SabineApp::OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                        CefRefPtr<CefFrame> frame,
                                        CefProcessId source_process,
                                        CefRefPtr<CefProcessMessage> message) {
  CEF_REQUIRE_RENDERER_THREAD();
  if (source_process != PID_BROWSER || !browser || !frame || !message) return false;
  return sabine_bridge::Receive(browser, frame, message,
                                BridgePolicyFor(browser));
}

bool SabineApp::OnAlreadyRunningAppRelaunch(
    CefRefPtr<CefCommandLine> command_line,
    const CefString& current_directory) {
  CEF_REQUIRE_UI_THREAD();
  CreateBrowser(command_line);
  return true;
}

CefRefPtr<CefClient> SabineApp::GetDefaultClient() {
  return SabineOsrHandler::GetInstance();
}
