import type { SettingsPatch } from '../bindings'

/** `SettingsPatch` has every field (`null` = leave unchanged); this fills the ones a caller doesn't touch. */
export function settingsPatch(p: Partial<SettingsPatch>): SettingsPatch {
  return {
    appearance: null,
    model_idle_timeout: null,
    performance: null,
    input_device: null,
    recording_retention: null,
    default_language: null,
    default_voice: null,
    default_speed: null,
    tts_quality: null,
    ...p,
  }
}
