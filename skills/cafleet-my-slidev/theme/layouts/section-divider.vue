<script setup>
defineProps({
  section: { type: [String, Number], default: '' },
  totalSections: { type: [String, Number], default: 0 },
})
</script>

<template>
  <div class="slidev-layout section-divider">
    <div class="section-main">
      <div v-if="section" class="section-number">{{ section }}</div>
      <slot />
    </div>
    <div v-if="totalSections > 0" class="progress-bar">
      <div
        v-for="i in Number(totalSections)"
        :key="i"
        class="progress-dot"
        :class="{ active: i === Number(section), past: i < Number(section) }"
      />
    </div>
  </div>
</template>

<style scoped>
.section-divider {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  background: var(--c-bg);
  position: relative;
}

.section-main {
  text-align: center;
  padding: 48px;
}

.section-number {
  font-size: 6rem;
  font-weight: 800;
  color: var(--c-primary);
  opacity: 0.15;
  line-height: 1;
  margin-bottom: 8px;
  font-variant-numeric: tabular-nums;
}

.section-main :deep(h1) {
  font-weight: 800;
  font-size: 2.5rem;
  color: var(--c-text);
  margin-bottom: 12px;
}

.section-main :deep(p) {
  font-size: 1.1rem;
  font-weight: 500;
  color: var(--c-text-secondary);
  max-width: 600px;
  margin: 0 auto;
  line-height: 1.6;
}

/* Bullet list for section preview points */
.section-main :deep(ul) {
  text-align: left;
  max-width: 500px;
  margin: 16px auto 0;
}

.section-main :deep(ul > li) {
  font-size: 0.95rem;
  font-weight: 500;
  color: var(--c-text-secondary);
  margin-bottom: 4px;
}

.section-main :deep(ul > li::before) {
  width: 6px;
  height: 6px;
  top: 0.6em;
}

/* Progress indicator */
.progress-bar {
  position: absolute;
  bottom: 28px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  gap: 12px;
}

.progress-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--c-border);
  transition: all 0.3s;
}

.progress-dot.active {
  background: var(--c-primary);
  transform: scale(1.3);
}

.progress-dot.past {
  background: var(--c-primary);
  opacity: 0.4;
}

.section-divider::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 4px;
  background: var(--c-primary);
}
</style>
