<script setup>
defineProps({
  stats: { type: Array, default: () => [] },
})
</script>

<template>
  <div class="slidev-layout stats-grid-layout">
    <div class="stats-header">
      <slot name="header" />
    </div>
    <div class="stats-cards" :class="`cols-${Math.min(stats.length, 4)}`">
      <div v-for="(stat, i) in stats" :key="i" class="stat-card" :class="stat.type || 'primary'">
        <div class="stat-value">{{ stat.value }}</div>
        <div class="stat-label">{{ stat.label }} <span v-if="stat.source" class="stat-source">{{ stat.source }}</span></div>
      </div>
    </div>
    <slot />
    <div class="page-number">
      {{ String($slidev.nav.currentPage).padStart(String($slidev.nav.total).length, '0') }}/{{ String($slidev.nav.total).padStart(String($slidev.nav.total).length, '0') }}
    </div>
  </div>
</template>

<style scoped>
.stats-grid-layout {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--c-bg);
  position: relative;
}

.stats-header {
  border-left: 4px solid var(--c-primary);
  padding: 24px 48px 24px 44px;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
}

.stats-header :deep(h1) {
  font-weight: 800;
  font-size: 28px;
  color: var(--c-text);
  margin: 0;
}

.stats-cards {
  display: grid;
  gap: 24px;
  padding: 24px 48px;
  flex: 1;
  align-content: center;
}

.stats-cards.cols-2 { grid-template-columns: repeat(2, minmax(0, 1fr)); }
.stats-cards.cols-3 { grid-template-columns: repeat(3, minmax(0, 1fr)); }
.stats-cards.cols-4 { grid-template-columns: repeat(4, minmax(0, 1fr)); }

.stat-card {
  background: var(--c-primary-light);
  border-radius: 12px;
  padding: 28px 24px;
  text-align: center;
  border-left: 4px solid var(--c-primary);
  container-type: inline-size;
}

.stat-card.accent {
  background: var(--c-accent-light);
  border-left-color: var(--c-accent);
}
.stat-card.positive {
  background: var(--c-positive-light);
  border-left-color: var(--c-positive);
}
.stat-card.negative {
  background: var(--c-negative-light);
  border-left-color: var(--c-negative);
}
.stat-card.important {
  background: var(--c-important-light);
  border-left-color: var(--c-important);
}

.stat-value {
  font-size: clamp(2rem, 5cqi, 4rem);
  font-weight: 800;
  line-height: var(--lh-tight);
  color: var(--c-primary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  margin-bottom: 8px;
}

.stat-card.accent .stat-value { color: var(--c-accent); }
.stat-card.positive .stat-value { color: var(--c-positive); }
.stat-card.negative .stat-value { color: var(--c-negative); }
.stat-card.important .stat-value { color: var(--c-important); }

.stat-label {
  font-size: 1rem;
  font-weight: 700;
  color: var(--c-text);
  line-height: 1.4;
}

.stat-source {
  font-size: 0.75rem;
  color: var(--c-text-secondary);
}

.page-number {
  position: absolute;
  bottom: 16px;
  right: 32px;
  font-size: 13px;
  color: var(--c-text-secondary);
  font-weight: 500;
}

.stats-grid-layout::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 4px;
  background: var(--c-primary);
}
</style>
