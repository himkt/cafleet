<script setup>
import { computed } from 'vue'

const props = defineProps({
  columns: {
    type: String,
    default: '1:1',
  },
  fontSize: {
    type: String,
    default: '',
  },
})

const gridTemplate = computed(() => {
  const parts = props.columns.split(':').map(Number)
  return parts.map(p => `${p}fr`).join(' ')
})
</script>

<template>
  <div class="slidev-layout two-cols">
    <div class="two-cols-header">
      <slot name="header" />
    </div>
    <div class="two-cols-content" :style="{ gridTemplateColumns: gridTemplate, ...(fontSize ? { fontSize } : {}) }">
      <div class="two-cols-left">
        <slot name="left" />
      </div>
      <div class="two-cols-right">
        <slot name="right" />
      </div>
    </div>
    <div class="page-number">
      {{ String($slidev.nav.currentPage).padStart(String($slidev.nav.total).length, '0') }}/{{ String($slidev.nav.total).padStart(String($slidev.nav.total).length, '0') }}
    </div>
  </div>
</template>

<style scoped>
.two-cols {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--c-bg);
  position: relative;
}

.two-cols-header {
  background: transparent;
  border-left: 4px solid var(--c-primary);
  padding: 22px 48px 22px 44px;
  flex: 0 0 auto;
  min-height: 10%;
  display: flex;
  align-items: center;
}

.two-cols-header :deep(h1) {
  font-weight: 800;
  font-size: 30px;
  color: var(--c-text);
  margin: 0;
  line-height: 1.15;
}

/* Column subheading: when the first paragraph in a column starts with a strong tag,
   treat it as a section label so it is visually distinct from the bullet text below. */
.two-cols-content :deep(> div > p:first-child) {
  font-size: 1.15em;
  font-weight: 800;
  color: var(--c-text);
  margin: 0 0 14px 0;
  letter-spacing: -0.01em;
}

.two-cols-content {
  display: grid;
  gap: 32px;
  padding: 32px 48px;
  flex: 1;
}

/* Top-level bullets in either column: bold, primary text, slightly tighter rhythm than single-column
   since each column is narrower and reads denser. Bullet markers come from index.css ::before rules. */
.two-cols-content :deep(> div > ul > li),
.two-cols-content :deep(> div > ol > li) {
  font-weight: 700;
  color: var(--c-text);
  margin-bottom: 10px;
  line-height: 1.35;
}

/* When a parent has a nested list, drop its trailing margin so the visual rhythm comes from the
   nested-ul's bottom margin instead — avoids the "double gap" between sub-bullets and the next parent. */
.two-cols-content :deep(li:has(> ul)) {
  margin-bottom: 5px;
}

.two-cols-content :deep(li > ul) {
  padding-left: 1.1em;
  margin-top: 5px;
  margin-bottom: 8px;
}

/* Nested bullets: medium weight, slightly smaller, tighter spacing, neutral-700 color. */
.two-cols-content :deep(li li) {
  font-weight: 600;
  font-size: 0.9em;
  color: var(--c-text-secondary);
  margin-bottom: 5px;
  line-height: 1.35;
}

.two-cols-content :deep(li li:last-child) {
  margin-bottom: 0;
}

.two-cols-content :deep(li li li) {
  font-size: 0.92em;
  font-weight: 500;
}

.page-number {
  position: absolute;
  bottom: 16px;
  right: 32px;
  font-size: 13px;
  color: var(--c-text-secondary);
  font-weight: 500;
}

.two-cols::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 4px;
  background: var(--c-primary);
}
</style>
