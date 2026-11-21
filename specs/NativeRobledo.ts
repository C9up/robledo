import type {TurboModule} from 'react-native';
import {TurboModuleRegistry} from 'react-native';

/** robledo's client on the phone (Rust, behind a C++ JSI module). Mirrors `NativeRobledo` in src/mobile. */
export interface Spec extends TurboModule {
  /** The app's update server, runtime, platform, and the current and pending update ids, as JSON. */
  readonly status: () => string;
  /** Verify an update and keep it for the next start; returns its id, throws why it was refused. */
  readonly stage: (manifest: string, signature: string, paths: ReadonlyArray<string>, files: ReadonlyArray<Object>) => string;
  /** The app showed its first screen on the current update. */
  readonly ready: () => void;
}

export default TurboModuleRegistry.getEnforcing<Spec>('NativeRobledo');
