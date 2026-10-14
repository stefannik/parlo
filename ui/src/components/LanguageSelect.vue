<script setup lang="ts">
import { LANGUAGES } from '../lib/languages'

defineProps<{ modelValue: string; label?: string; compact?: boolean }>()
defineEmits<{ 'update:modelValue': [string] }>()
</script>

<template>
  <label class="lang" :class="{ compact }">
    <span v-if="label" class="small muted">{{ label }}</span>
    <select :value="modelValue" :aria-label="label ?? 'Language'" data-testid="language-select" @change="$emit('update:modelValue', ($event.target as HTMLSelectElement).value)">
      <option v-for="l in LANGUAGES" :key="l.code" :value="l.code">{{ l.name }}</option>
    </select>
  </label>
</template>

<style scoped>
.lang { display: inline-flex; align-items: center; gap: var(--s2); }
select {
  appearance: none; -webkit-appearance: none; min-height: 28px; padding: 0 28px 0 12px; border-radius: var(--r-sm);
  border: 1px solid var(--border); color: var(--text);
  background: var(--surface) url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='m1 1 4 4 4-4' fill='none' stroke='%23888' stroke-width='1.5' stroke-linecap='round'/%3E%3C/svg%3E") no-repeat right 10px center;
  box-shadow: var(--shadow-1);
}
select:hover { background-color: var(--surface-hover); }
.compact select { min-height: 26px; font-size: 12px; }
</style>
