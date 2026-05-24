<script setup>
defineProps({
  fontSize: { type: String, default: '18px' },
})
</script>

<template>
  <div class="slidev-layout bullets">
    <div class="bullets-header">
      <slot name="header" />
    </div>
    <div class="bullets-content" :style="{ fontSize }">
      <slot />
    </div>
    <div class="page-number">
      {{ String($slidev.nav.currentPage).padStart(String($slidev.nav.total).length, '0') }}/{{ String($slidev.nav.total).padStart(String($slidev.nav.total).length, '0') }}
    </div>
  </div>
</template>

<style scoped>
.bullets {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--c-bg);
  position: relative;
}

.bullets-header {
  background: transparent;
  border-left: 4px solid var(--c-primary);
  padding: 22px 48px 22px 44px;
  flex: 0 0 auto;
  min-height: 10%;
  display: flex;
  align-items: center;
}

.bullets-header :deep(h1) {
  font-weight: 800;
  font-size: 30px;
  color: var(--c-text);
  margin: 0;
  line-height: 1.15;
}

.bullets-content {
  padding: 32px 48px;
  flex: 1;
}

/* Top-level bullets: bold, primary text color, generous spacing.
   Bullet markers themselves come from the global theme's ::before rules — do NOT add list-style-type here. */
.bullets-content :deep(> ul > li),
.bullets-content :deep(> ol > li) {
  font-weight: 700;
  color: var(--c-text);
  margin-bottom: 16px;
  line-height: 1.4;
}

/* When a parent has a nested list, drop its trailing margin so the visual rhythm comes from the
   nested-ul's bottom margin instead — avoids the "double gap" between sub-bullets and the next parent. */
.bullets-content :deep(li:has(> ul)) {
  margin-bottom: 8px;
}

/* The nested list itself contributes the gap to the next sibling parent. */
.bullets-content :deep(li > ul) {
  padding-left: 1.1em;
  margin-top: 8px;
  margin-bottom: 12px;
}

/* Nested bullets: medium weight, slightly smaller, comfortable spacing, neutral-700 color for clear hierarchy. */
.bullets-content :deep(li li) {
  font-weight: 600;
  font-size: 0.9em;
  color: var(--c-text-secondary);
  margin-bottom: 8px;
  line-height: 1.4;
}

.bullets-content :deep(li li:last-child) {
  margin-bottom: 0;
}

/* 3rd-level: even lighter weight */
.bullets-content :deep(li li li) {
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

.bullets::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 4px;
  background: var(--c-primary);
}
</style>
