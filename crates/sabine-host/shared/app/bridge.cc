#include "app/bridge.h"
#include <map>
#include <utility>
#include "common/bridge_policy.h"
#include "common/json.h"
#include "osr/utilities.h"

namespace sabine_bridge {
namespace {
struct PendingRequest {
  CefRefPtr<CefV8Context> context;
  std::string page_id;
  CefRefPtr<CefBrowser> browser;
};
std::map<std::string, PendingRequest> pending;
struct ContextOrigin {
  CefRefPtr<CefV8Context> context;
  std::string origin;
};
std::vector<ContextOrigin> contexts;
const std::string process_token = UniqueToken();
uint64_t next_request = 0;
constexpr size_t kMaxPendingRequests = 128;
constexpr size_t kMaxMessageBytes = 1024 * 1024;

class NativePostMessageHandler : public CefV8Handler {
 public:
  NativePostMessageHandler(CefRefPtr<CefFrame> frame, std::string message_name, std::string origin)
      : frame_(frame), message_name_(std::move(message_name)), origin_(std::move(origin)) {}

  bool Execute(const CefString& name, CefRefPtr<CefV8Value> object,
               const CefV8ValueList& arguments, CefRefPtr<CefV8Value>& retval,
               CefString& exception) override {
    if (arguments.size() != 1 || !arguments[0]->IsString()) {
      exception = "Sabine native messages require one string argument";
      return true;
    }
    std::string payload = arguments[0]->GetStringValue();
    if (payload.size() > kMaxMessageBytes) {
      exception = "Sabine native message exceeds 1 MiB";
      return true;
    }
    auto context = CefV8Context::GetCurrentContext();
    if (!context || !context->IsValid() || !frame_->IsValid()) return true;
    auto browser = frame_->GetBrowser();
    const std::string cancel_prefix = "sabine://cancel/";
    if (message_name_ == "sabine.native" && payload.rfind(cancel_prefix, 0) == 0) {
      const std::string id = payload.substr(cancel_prefix.size());
      for (auto it = pending.begin(); it != pending.end();) {
        if (it->second.browser->IsSame(browser) && it->second.page_id == id &&
            it->second.context->IsSame(context)) it = pending.erase(it);
        else ++it;
      }
      return true;
    }
    const std::string id = sabine_osr::BridgeRequestId(payload);
    if (message_name_ == "sabine.native" && !id.empty()) {
      if (pending.size() >= kMaxPendingRequests) {
        exception = "Sabine bridge request capacity is exhausted";
        return true;
      }
      const std::string native_id = process_token + "-" + std::to_string(++next_request);
      const size_t query = payload.find('?');
      if (query == std::string::npos) {
        exception = "Malformed Sabine bridge request";
        return true;
      }
      pending.emplace(native_id, PendingRequest{context, id, browser});
      payload = "sabine://bridge/" + native_id + payload.substr(query);
    }
    auto message = CefProcessMessage::Create(message_name_);
    message->GetArgumentList()->SetString(0, payload);
    if (message_name_ == "sabine.native") message->GetArgumentList()->SetString(1, origin_);
    frame_->SendProcessMessage(PID_BROWSER, message);
    retval = CefV8Value::CreateUndefined();
    return true;
  }
 private:
  CefRefPtr<CefFrame> frame_;
  std::string message_name_;
  std::string origin_;
  IMPLEMENT_REFCOUNTING(NativePostMessageHandler);
};
}

void InstallTransport(CefRefPtr<CefFrame> frame, CefRefPtr<CefV8Context> context,
                      const char* name, const char* message) {
  context->GetGlobal()->SetValue(name,
      CefV8Value::CreateFunction(name, new NativePostMessageHandler(frame, message, context->GetGlobal()->GetValue("origin")->GetStringValue())),
      static_cast<CefV8Value::PropertyAttribute>(V8_PROPERTY_ATTRIBUTE_READONLY | V8_PROPERTY_ATTRIBUTE_DONTDELETE));
}

std::string RememberContext(CefRefPtr<CefV8Context> context) {
  const std::string origin = context->GetGlobal()->GetValue("origin")->GetStringValue();
  contexts.push_back({context, origin});
  return origin;
}

void ReleaseContext(CefRefPtr<CefV8Context> context) {
  for (auto it = contexts.begin(); it != contexts.end();) {
    if (it->context->IsSame(context)) it = contexts.erase(it);
    else ++it;
  }
  for (auto it = pending.begin(); it != pending.end();) {
    if (it->second.context->IsSame(context)) it = pending.erase(it);
    else ++it;
  }
}

void ReleaseBrowser(CefRefPtr<CefBrowser> browser) {
  for (auto it = pending.begin(); it != pending.end();) {
    if (it->second.browser->IsSame(browser)) it = pending.erase(it);
    else ++it;
  }
}

bool Receive(CefRefPtr<CefBrowser> browser, CefRefPtr<CefFrame> frame,
             CefRefPtr<CefProcessMessage> message,
             CefRefPtr<CefDictionaryValue> policy) {
  const std::string name = message->GetName();
  auto values = message->GetArgumentList();
  CefRefPtr<CefV8Context> context;
  std::string script;
  if (name == "sabine.response") {
    if (values->GetSize() != 3) return true;
    const std::string id = values->GetString(0);
    auto found = pending.find(id);
    if (found == pending.end() || !found->second.browser->IsSame(browser)) return true;
    context = found->second.context;
    const std::string page_id = found->second.page_id;
    pending.erase(found);
    script = "window.__sabineBridgeResolve&&window.__sabineBridgeResolve(" +
        JsString(page_id) + "," + (values->GetBool(1) ? "true" : "false") + ",JSON.parse(" +
        JsString(values->GetString(2)) + "));";
  } else if (name == "sabine.event") {
    if (values->GetSize() != 2 || !frame->IsMain() ||
        !AllowsDocument(policy, frame->GetURL())) return true;
    context = frame->GetV8Context();
    if (!context || !context->IsValid()) return true;
    bool authorized = false;
    for (const auto& entry : contexts) {
      if (entry.context->IsSame(context)) {
        authorized = MatchesSecurityOrigin(policy, frame->GetURL(), entry.origin);
        break;
      }
    }
    if (!authorized) return true;
    script = "window.__sabineBridgeEmit&&window.__sabineBridgeEmit(JSON.parse(" +
        JsString(values->GetString(0)) + "),JSON.parse(" +
        JsString(values->GetString(1)) + "));";
  } else {
    return false;
  }
  if (context && context->IsValid()) {
    CefRefPtr<CefV8Value> result;
    CefRefPtr<CefV8Exception> exception;
    context->Eval(script, "", 0, result, exception);
  }
  return true;
}
}
