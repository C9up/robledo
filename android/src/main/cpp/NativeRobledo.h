#pragma once

#include <AppSpecsJSI.h>

#include <memory>
#include <string>

namespace facebook::react {

/** robledo's JSI module: status, staging and readiness, over its Rust client. */
class NativeRobledo : public NativeRobledoCxxSpec<NativeRobledo> {
 public:
  explicit NativeRobledo(std::shared_ptr<CallInvoker> jsInvoker);

  std::string status(jsi::Runtime& rt);
  std::string stage(jsi::Runtime& rt, std::string manifest, std::string signature, jsi::Array paths, jsi::Array files);
  void ready(jsi::Runtime& rt);
};

} // namespace facebook::react
