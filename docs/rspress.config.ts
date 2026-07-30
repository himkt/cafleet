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
    nav: [{ text: 'Docs', link: '/quickstart' }],
    socialLinks: [
      { icon: 'github', mode: 'link', content: 'https://github.com/himkt/cafleet' },
    ],
  },
});
