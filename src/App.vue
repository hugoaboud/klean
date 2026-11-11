<script setup lang="ts">
import { onMounted } from "vue";
import ProjectPage from "./pages/ProjectPage.vue";
import WelcomePage from "./pages/WelcomePage.vue";
import { actions, state } from "./store/workspace";

onMounted(() => {
  void actions.bootstrap();
});
</script>

<template>
  <div class="app">
    <WelcomePage v-if="state.page === 'welcome'" />
    <ProjectPage v-else />
  </div>
</template>

<style scoped>
.app {
  display: contents;
}
</style>
