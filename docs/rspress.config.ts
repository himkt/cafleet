import path from 'node:path';
import { defineConfig } from '@rspress/core';

export default defineConfig({
  root: 'docs',
  base: '/cafleet/',
  globalStyles: path.join(__dirname, 'theme/styles.css'),
  title: 'CAFleet',
  description: 'Message broker and member registry for coding agents.',
  llms: true,
  markdown: {
    checkDeadLinks: true,
  },
  themeConfig: {
    socialLinks: [
      { icon: 'github', mode: 'link', content: 'https://github.com/himkt/cafleet' },
    ],
  },
});
