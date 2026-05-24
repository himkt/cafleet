<script setup>
import { computed } from 'vue'

const props = defineProps({
  type: {
    type: String,
    default: 'note',
    validator: (v) => ['note', 'important', 'tip', 'warning', 'caution', 'formula'].includes(v),
  },
  title: {
    type: String,
    default: '',
  },
})

const displayTitle = computed(() => {
  if (props.title) return props.title
  return props.type.charAt(0).toUpperCase() + props.type.slice(1)
})
</script>

<template>
  <div class="admonition" :class="`admonition-${type}`">
    <div class="admonition-title">{{ displayTitle }}</div>
    <div class="admonition-content">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.admonition {
  border-left: 4px solid var(--c-primary);
  background: var(--c-primary-light);
  padding: 12px 16px;
  border-radius: 0 6px 6px 0;
  margin: 8px 0;
}

.admonition-title {
  font-weight: 700;
  font-size: 0.9rem;
  color: var(--c-primary);
  margin-bottom: 4px;
}

.admonition-content {
  font-weight: 400;
  font-size: 0.85rem;
  color: var(--c-text);
}

/* note — default, uses primary (blue) */
.admonition-note {
  border-left-color: var(--c-primary);
  background: var(--c-primary-light);
}
.admonition-note .admonition-title {
  color: var(--c-primary);
}

/* important — purple */
.admonition-important {
  border-left-color: var(--c-important);
  background: var(--c-important-light);
}
.admonition-important .admonition-title {
  color: var(--c-important);
}

/* tip — green */
.admonition-tip {
  border-left-color: var(--c-positive);
  background: var(--c-positive-light);
}
.admonition-tip .admonition-title {
  color: var(--c-positive);
}

/* warning — orange */
.admonition-warning {
  border-left-color: var(--c-accent);
  background: var(--c-accent-light);
}
.admonition-warning .admonition-title {
  color: var(--c-accent);
}

/* caution — red */
.admonition-caution {
  border-left-color: var(--c-negative);
  background: var(--c-negative-light);
}
.admonition-caution .admonition-title {
  color: var(--c-negative);
}

/* formula — blue, optimized for mathematical content */
.admonition-formula {
  border-left-color: var(--c-primary);
  background: var(--c-primary-light);
}
.admonition-formula .admonition-title {
  color: var(--c-primary);
}
.admonition-formula .admonition-content {
  font-size: 1rem;
}
</style>
