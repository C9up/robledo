import type {TurboModule} from 'react-native';
import {TurboModuleRegistry} from 'react-native';

/** robledo's client on the phone (Rust, behind a C++ JSI module). Mirrors `NativeRobledo` in src/mobile. */
export interface Spec extends TurboModule {
  /** The app's update server, runtime, platform, and the current and pending update ids, as JSON. */
  readonly status: () => string;
  /** Check an update's manifest and signature before any file is fetched; returns `{"id", "staging"}` as JSON, throws why it was refused. */
  readonly verify: (manifest: string, signature: string) => string;
  /** One file of the update `verify` accepted (an ArrayBuffer or a Uint8Array), checked against its signed size and digest. */
  readonly receive: (path: string, file: Object) => void;
  /** Every file arrived: keep the update for the next start; returns its id. */
  readonly finish: () => string;
  /** The app showed its first screen on the current update. */
  readonly ready: () => void;
}

export default TurboModuleRegistry.getEnforcing<Spec>('NativeRobledo');
