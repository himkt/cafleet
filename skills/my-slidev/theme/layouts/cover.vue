<script setup>
defineProps({
  fontSize: { type: String, default: '22px' },
})
</script>

<template>
  <div class="slidev-layout cover">
    <div class="cover-content" :style="{ fontSize }">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.cover {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  background: var(--c-bg);
  position: relative;
}

.cover::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 4px;
  background: var(--c-primary);
}

.cover-content {
  text-align: center;
}

.cover-content :deep(h1) {
  font-weight: 800;
  font-size: 40px;
  color: var(--c-text);
  margin-bottom: 16px;
}

.cover-content :deep(p) {
  font-weight: 700;
  color: var(--c-text-secondary);
}
</style>
