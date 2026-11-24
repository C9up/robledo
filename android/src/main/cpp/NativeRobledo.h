#pragma once

#include <AppSpecsJSI.h>

#include <memory>
#include <string>

namespace facebook::react {

/** robledo's JSI module: status, checking and receiving an update, readiness, over its Rust client. */
class NativeRobledo : public NativeRobledoCxxSpec<NativeRobledo> {
 public:
  explicit NativeRobledo(std::shared_ptr<CallInvoker> jsInvoker);

  std::string status(jsi::Runtime& rt);
  std::string verify(jsi::Runtime& rt, std::string manifest, std::string signature);
  void receive(jsi::Runtime& rt, std::string path, jsi::Object file);
  std::string finish(jsi::Runtime& rt);
  void ready(jsi::Runtime& rt);
};

} // namespace facebook::react
