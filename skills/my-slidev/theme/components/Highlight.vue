<script setup>
defineProps({
  type: {
    type: String,
    default: 'primary',
    validator: (v) => ['primary', 'positive', 'negative', 'accent', 'important'].includes(v),
  },
})
</script>

<template>
  <span class="highlight" :class="`highlight-${type}`">
    <slot />
  </span>
</template>

<style scoped>
.highlight {
  padding: 0.1em 0.35em;
  border-radius: 0.25em;
  font-weight: 600;
}

.highlight-primary {
  background: oklch(90% 0.05 263);
  color: var(--c-primary);
}

.highlight-positive {
  background: oklch(90% 0.05 149);
  color: var(--c-positive);
}

.highlight-negative {
  background: oklch(90% 0.05 27);
  color: var(--c-negative);
}

.highlight-accent {
  background: oklch(90% 0.05 48);
  color: var(--c-accent);
}

.highlight-important {
  background: oklch(90% 0.05 293);
  color: var(--c-important);
}
</style>
