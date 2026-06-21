<script setup>
defineProps({
  fontSize: { type: String, default: '14px' },
})
</script>

<template>
  <div class="slidev-layout bullets-sm">
    <div class="bullets-sm-header">
      <slot name="header" />
    </div>
    <div class="bullets-sm-content" :style="{ fontSize }">
      <slot />
    </div>
    <div class="page-number">
      {{ String($slidev.nav.currentPage).padStart(String($slidev.nav.total).length, '0') }}/{{ String($slidev.nav.total).padStart(String($slidev.nav.total).length, '0') }}
    </div>
  </div>
</template>

<style scoped>
.bullets-sm {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--c-bg);
  position: relative;
}

.bullets-sm-header {
  background: transparent;
  border-left: 4px solid var(--c-primary);
  padding: 14px 48px 14px 44px;
  flex: 0 0 auto;
  min-height: 10%;
  display: flex;
  align-items: center;
}

.bullets-sm-header :deep(h1) {
  font-weight: 800;
  font-size: 26px;
  color: var(--c-text);
  margin: 0;
  line-height: 1.15;
}

.bullets-sm-content {
  padding: 14px 48px;
  flex: 1;
}

.bullets-sm-content :deep(ul) {
  list-style: none;
  padding-left: 0;
}

.bullets-sm-content :deep(li) {
  font-weight: 500;
  color: var(--c-text);
  margin-bottom: 6px;
  padding-left: 0;
}

.bullets-sm-content :deep(li::before) {
  display: none;
}

/* Nested sub-bullets in bullets-sm: lighter, smaller, tighter spacing, gently indented */
.bullets-sm-content :deep(li ul) {
  padding-left: 1em;
  margin-top: 3px;
}

.bullets-sm-content :deep(li li) {
  font-weight: 400;
  font-size: 0.9em;
  color: var(--c-text-secondary);
  margin-bottom: 3px;
  line-height: 1.3;
}

.page-number {
  position: absolute;
  bottom: 16px;
  right: 32px;
  font-size: 13px;
  color: var(--c-text-secondary);
  font-weight: 500;
}

.bullets-sm::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 4px;
  background: var(--c-primary);
}
</style>
