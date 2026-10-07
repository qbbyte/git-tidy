<script setup lang="ts">
import { NConfigProvider, NMessageProvider, dateZhCN, zhCN, type GlobalThemeOverrides } from "naive-ui";
import { RouterView } from "vue-router";

const themeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#1F5AA8",
    primaryColorHover: "#2C6FC4",
    primaryColorPressed: "#1A4C8F",
    borderRadius: "6px",
  },
};
</script>

<template>
  <n-config-provider :locale="zhCN" :date-locale="dateZhCN" :theme-overrides="themeOverrides">
    <n-message-provider>
      <div class="app-shell">
        <header class="app-bar">
          <span class="app-title">Git Tidy</span>
          <span class="app-subtitle">本地 Git 提交治理</span>
        </header>
        <main class="app-body">
          <router-view />
        </main>
      </div>
    </n-message-provider>
  </n-config-provider>
</template>

<style>
html,
body,
#app {
  height: 100%;
  margin: 0;
}
</style>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #f5f7fa;
}

.app-bar {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 12px 24px;
  background: #1f5aa8;
  color: #fff;
}

.app-title {
  font-size: 16px;
  font-weight: 600;
}

.app-subtitle {
  font-size: 12px;
  opacity: 0.75;
}

.app-body {
  flex: 1;
  overflow: auto;
  padding: 20px 24px;
}
</style>
