<script setup lang="ts">
import { computed } from 'vue'
import { useData } from 'vitepress'

type Version = { version: string; released: boolean; pages: { name: string }[] }

const { page, theme } = useData()
const versions = computed<Version[]>(() => theme.value.versions)
const location = computed(() => {
  const [version, file = ''] = page.value.relativePath.split('/')
  return { version, name: file.replace(/\.md$/, '') }
})
const current = computed(() => versions.value.find(({ version }) => version === location.value.version))
const newest = computed(() => versions.value[0])
// The same page in the newest version, or its specification if the page is gone.
const newer = computed(() => {
  const { name } = location.value
  const page = newest.value.pages.some((entry) => entry.name === name) ? name : 'convention'
  return `/${newest.value.version}/${page}`
})
</script>

<template>
  <p v-if="current" class="version-banner">
    <template v-if="!current.released">Unreleased text</template>
    <template v-else>Version {{ current.version }}</template>
    <template v-if="current !== newest">.
      The newest version is <a :href="newer">{{ newest.version }}</a>.</template>
  </p>
</template>
