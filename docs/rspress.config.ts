import { defineConfig } from '@rspress/core';

export default defineConfig({
  root: 'docs',
  base: '/cafleet/',
  title: 'CAFleet',
  description: 'Message broker and member registry for coding agents.',
  markdown: {
    checkDeadLinks: true,
  },
  themeConfig: {
    socialLinks: [
      { icon: 'github', mode: 'link', content: 'https://github.com/himkt/cafleet' },
    ],
  },
});
