<script setup lang="ts">
import { onMounted } from "vue";
import ParseModal from "./components/ParseModal.vue";
import ProjectPage from "./pages/ProjectPage.vue";
import SplashPage from "./pages/SplashPage.vue";
import WelcomePage from "./pages/WelcomePage.vue";
import { actions, state } from "./store/workspace";

onMounted(() => {
  // The splash page does the startup work (Lean's parser and the editor's code)
  // and calls back when it is done; the welcome page then has nothing to wait for.
  void actions.listen();
});
</script>

<template>
  <div class="app">
    <SplashPage v-if="state.page === 'splash'" />
    <WelcomePage v-else-if="state.page === 'welcome'" />
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
