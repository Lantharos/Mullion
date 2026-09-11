#ifndef SABINE_BRIDGE_POLICY_H_
#define SABINE_BRIDGE_POLICY_H_

#include <string>
#include <random>
#include <sstream>
#include <iomanip>
#include <vector>
#include "include/cef_parser.h"
#include "include/cef_values.h"

namespace sabine_bridge {
inline std::string UniqueToken() {
  std::random_device random;
  std::ostringstream token;
  for (int index = 0; index < 4; ++index) {
    token << std::hex << std::setfill('0') << std::setw(8) << static_cast<uint32_t>(random());
  }
  return token.str();
}

inline std::string TrustedHtmlUrl(CefRefPtr<CefDictionaryValue> policy,
                                  const std::string& html) {
  return policy->GetString("htmlPrefix").ToString() +
         CefBase64Encode(html.data(), html.size()).ToString();
}

inline std::string DocumentUrl(const std::string& url) {
  CefURLParts parts;
  if (!CefParseURL(url, parts)) return "";
  const std::string canonical = CefString(&parts.spec);
  return canonical.substr(0, canonical.find_first_of("?#"));
}

inline std::string Origin(const std::string& url) {
  CefURLParts parts;
  if (!CefParseURL(url, parts)) return "";
  const std::string scheme = CefString(&parts.scheme);
  if (scheme != "http" && scheme != "https") return "";
  return CefString(&parts.origin);
}

inline bool MatchesOrigin(const std::string& origin,
                          CefRefPtr<CefListValue> allowed) {
  if (origin.empty() || !allowed) return false;
  for (size_t index = 0; index < allowed->GetSize(); ++index) {
    const std::string candidate = allowed->GetString(index);
    if (candidate == "*") return true;
    CefURLParts parts;
    if (!CefParseURL(candidate, parts)) continue;
    const std::string path = CefString(&parts.path);
    if ((path.empty() || path == "/") && CefString(&parts.query).empty() &&
        CefString(&parts.fragment).empty() && CefString(&parts.username).empty() &&
        CefString(&parts.password).empty() && Origin(candidate) == origin) return true;
  }
  return false;
}

inline bool AllowsDocument(CefRefPtr<CefDictionaryValue> policy,
                           const std::string& url) {
  if (!policy || !policy->GetBool("enabled")) return false;
  const std::string prefix = policy->GetString("htmlPrefix");
  if (!prefix.empty() && url.rfind(prefix, 0) == 0) return true;
  const std::string document = policy->GetString("document");
  if (!document.empty() && DocumentUrl(url) == DocumentUrl(document)) return true;
  return MatchesOrigin(Origin(url), policy->GetList("origins"));
}

inline bool MatchesSecurityOrigin(CefRefPtr<CefDictionaryValue> policy,
                                   const std::string& url,
                                   const std::string& security_origin) {
  const auto origin = Origin(url);
  return origin.empty() ? AllowsDocument(policy, url)
                        : origin == Origin(security_origin);
}

inline bool AllowsCommand(CefRefPtr<CefDictionaryValue> policy,
                          const std::string& url, const std::string& command) {
  if (!policy || !policy->GetBool("enabled")) return false;
  if (AllowsDocument(policy, url)) return true;
  auto origins = policy->GetDictionary("commandOrigins");
  return origins && MatchesOrigin(Origin(url), origins->GetList(command));
}

inline bool ExposesBridge(CefRefPtr<CefDictionaryValue> policy,
                          const std::string& url) {
  if (!policy || !policy->GetBool("enabled")) return false;
  if (AllowsDocument(policy, url)) return true;
  auto origins = policy->GetDictionary("commandOrigins");
  if (!origins) return false;
  CefDictionaryValue::KeyList commands;
  origins->GetKeys(commands);
  for (const auto& command : commands) {
    if (MatchesOrigin(Origin(url), origins->GetList(command))) return true;
  }
  return false;
}

inline std::vector<std::string> Commands(CefRefPtr<CefDictionaryValue> policy) {
  std::vector<std::string> commands;
  auto values = policy ? policy->GetList("commands") : nullptr;
  if (values) {
    for (size_t index = 0; index < values->GetSize(); ++index) {
      commands.push_back(values->GetString(index));
    }
  }
  return commands;
}
}  // namespace sabine_bridge
#endif
