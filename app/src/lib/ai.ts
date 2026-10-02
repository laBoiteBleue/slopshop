// The AI components' display: names and failures, translated (ADR 0025).

import type { AiFailure } from "./engine";
import { t } from "./i18n/index.svelte";
import type { MessageKey } from "./i18n/en";

const NAMES: Record<string, MessageKey> = {
  "runtime-directml": "ai.component.runtimeDirectml",
  "runtime-coreml-macos-arm64": "ai.component.runtimeCoreml",
  "runtime-cpu-linux-x64": "ai.component.runtimeCpu",
  "runtime-cpu-linux-arm64": "ai.component.runtimeCpu",
  "sam2.1-base-plus-fp32": "ai.component.samBasePlus",
  "sam2.1-tiny": "ai.component.samTiny",
  "birefnet-fp32": "ai.component.birefnet",
  "birefnet-lite": "ai.component.birefnetLite",
  "sam2.1-base-plus": "ai.component.samBasePlus",
  "vitmatte-small": "ai.component.vitmatte",
  birefnet: "ai.component.birefnet",
};

/** A component's name; its id if this version of the UI does not know it. */
export function componentName(id: string): string {
  const key = NAMES[id];
  return key ? t(key) : id;
}

const FAILURES: Record<string, MessageKey> = {
  network: "ai.error.network",
  disk: "ai.error.disk",
  corrupt: "ai.error.corrupt",
  busy: "ai.error.busy",
  start: "ai.error.start",
  model: "ai.error.model",
  unsupported: "ai.unsupported",
};

/** What went wrong, for the user (`null` when the user cancelled). */
export function failureMessage(error: unknown): string | null {
  const failure = error as Partial<AiFailure> | null;
  if (failure?.code === "cancelled") return null;
  const key = (failure?.code && FAILURES[failure.code]) || "ai.error.internal";
  return t(key, { detail: failure?.detail ?? String(error) });
}
