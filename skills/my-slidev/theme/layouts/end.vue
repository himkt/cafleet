<script setup>
defineProps({
  fontSize: { type: String, default: '' },
})
</script>

<template>
  <div class="slidev-layout end-layout">
    <div class="end-content" :style="fontSize ? { fontSize } : {}">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.end-layout {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  background: var(--c-bg);
  position: relative;
}

.end-content {
  text-align: center;
  padding: 48px;
}

.end-content :deep(h1) {
  font-weight: 800;
  font-size: 2.5rem;
  color: var(--c-text);
  margin-bottom: 16px;
}

.end-content :deep(p) {
  font-size: 1.1rem;
  font-weight: 500;
  color: var(--c-text-secondary);
  line-height: 1.8;
}

.end-layout::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 4px;
  background: var(--c-primary);
}
</style>
