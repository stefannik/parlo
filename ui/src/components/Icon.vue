<script setup lang="ts">
// Tiny inline icon set (24×24 stroke icons) — no icon dependency, nothing loaded at runtime.
const props = withDefaults(defineProps<{ name: string; size?: number }>(), { size: 18 })
const PATHS: Record<string, string> = {
  mic: 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3Zm-6 8a6 6 0 0 0 12 0M12 17v4M9 21h6',
  stop: 'M7 7h10v10H7z',
  pause: 'M9 6v12M15 6v12',
  play: 'M8 5v14l11-7z',
  gear: 'M10.33 5.30L10.55 2.81L13.45 2.81L13.67 5.30L15.55 6.09L17.47 4.48L19.52 6.53L17.91 8.45L18.70 10.33L21.19 10.55L21.19 13.45L18.70 13.67L17.91 15.55L19.52 17.47L17.47 19.52L15.55 17.91L13.67 18.70L13.45 21.19L10.55 21.19L10.33 18.70L8.45 17.91L6.53 19.52L4.48 17.47L6.09 15.55L5.30 13.67L2.81 13.45L2.81 10.55L5.30 10.33L6.09 8.45L4.48 6.53L6.53 4.48L8.45 6.09ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z',
  file: 'M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5',
  upload: 'M12 16V5m0 0-4 4m4-4 4 4M5 19h14',
  download: 'M12 4v11m0 0-4-4m4 4 4-4M5 20h14',
  copy: 'M9 9h10v11H9zM5 15V4h10',
  check: 'm5 12.5 4.5 4.5L19 7.5',
  x: 'M6 6l12 12M18 6 6 18',
  chevron: 'm7 10 5 5 5-5',
  refresh: 'M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6',
  folder: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z',
  warn: 'M12 4 2.5 20h19zM12 10v4.5M12 17.5v.01',
  info: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM12 11v5M12 8v.01',
  trash: 'M5 7h14M9 7V4h6v3M7 7l1 13h8l1-13',
  plus: 'M12 5v14M5 12h14',
  speaker: 'M4 9v6h4l5 4V5L8 9zM16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12',
  wave: 'M3 12h2M7 8v8M11 5v14M15 8v8M19 10v4M21 12h0',
  text: 'M5 6h14M5 11h14M5 16h9',
  eye: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Zm10 3a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z',
}
</script>

<template>
  <svg :width="props.size" :height="props.size" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
    <path :d="PATHS[props.name] ?? ''" />
  </svg>
</template>
