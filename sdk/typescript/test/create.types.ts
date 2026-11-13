import type { Effect, Scope } from "effect"
import * as AgentDesk from "../src/index.js"
import * as EffectHex from "../src/effect.js"

// Compile-only consumer checks: never start a helper from a type assertion.
export function createTypes(
  low: AgentDesk.CreateOptions,
  ready: AgentDesk.TranscriberOptions,
  ambiguous: Omit<AgentDesk.CreateOptions, "model"> & { model?: AgentDesk.ModelId },
) {
  const host: Promise<AgentDesk.HexHost> = AgentDesk.create(low)
  const transcriber: Promise<AgentDesk.Transcriber> = AgentDesk.create(ready)
  const effectHost: Effect.Effect<EffectHex.Host, EffectHex.HexError, Scope.Scope> = EffectHex.create(low)
  const effectTranscriber: Effect.Effect<EffectHex.Transcriber, EffectHex.HexError, Scope.Scope> = EffectHex.create(ready)
  const invalid = { model: "fictional-model" }

  // @ts-expect-error A model-bound transcriber cannot masquerade as a low-level host.
  const widened: AgentDesk.CreateOptions = ready
  // @ts-expect-error Optional models cannot determine which resource is returned.
  AgentDesk.create(ambiguous)
  // @ts-expect-error Optional models cannot determine which resource is returned.
  EffectHex.create(ambiguous)
  // @ts-expect-error Invalid model variables must not fall back to the low-level overload.
  AgentDesk.create(invalid)
  // @ts-expect-error Invalid model variables must not fall back to the low-level overload.
  EffectHex.create(invalid)
  // @ts-expect-error The existing service layer deliberately provides only the low-level client.
  EffectHex.layer(ready)

  return { host, transcriber, effectHost, effectTranscriber, widened }
}
