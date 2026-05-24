<script setup>
defineProps({
  fontSize: { type: String, default: '' },
})
</script>

<template>
  <div class="slidev-layout blank" :style="fontSize ? { fontSize } : {}">
    <slot />
    <div class="page-number">
      {{ String($slidev.nav.currentPage).padStart(String($slidev.nav.total).length, '0') }}/{{ String($slidev.nav.total).padStart(String($slidev.nav.total).length, '0') }}
    </div>
  </div>
</template>

<style scoped>
.blank {
  height: 100%;
  padding: 48px;
  background: var(--c-bg);
  position: relative;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.blank.v-center {
  justify-content: center;
}

.blank :deep(img) {
  max-width: 100%;
  max-height: 280px;
  object-fit: contain;
  display: block;
  margin: 0 auto;
  flex-shrink: 1;
}

.blank :deep(li) {
  margin-bottom: 6px;
}

.page-number {
  position: absolute;
  bottom: 16px;
  right: 32px;
  font-size: 13px;
  color: var(--c-text-secondary);
  font-weight: 500;
}

.blank::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 4px;
  background: var(--c-primary);
}
</style>
