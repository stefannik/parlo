import type { WorkspacePatch } from '../bindings'

/** `WorkspacePatch` has every field (no Option<Option<_>> over IPC); this fills the ones a caller doesn't touch. */
export function wsPatch(p: Partial<WorkspacePatch>): WorkspacePatch {
  return { transcript: null, tts: null, clear_transcript: false, clear_tts: false, ...p }
}
