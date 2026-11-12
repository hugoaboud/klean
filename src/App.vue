<script setup lang="ts">
import { onMounted } from "vue";
import ParseModal from "./components/ParseModal.vue";
import ProjectPage from "./pages/ProjectPage.vue";
import WelcomePage from "./pages/WelcomePage.vue";
import { actions, state } from "./store/workspace";

onMounted(() => {
  void actions.bootstrap();
  // Toolchain installs report progress from Rust as they download.
  void actions.listen();
});
</script>

<template>
  <div class="app">
    <WelcomePage v-if="state.page === 'welcome'" />
    <ProjectPage v-else />
    <!-- Reading a project holds the page back: there is no board until it is done. -->
    <ParseModal />
  </div>
</template>

<style scoped>
.app {
  display: contents;
}
</style>
