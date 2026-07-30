import { HomeLayout as BasicHomeLayout } from '@rspress/core/theme-original';
import { Content } from '@rspress/core/runtime';

function HomeLayout() {
  return <BasicHomeLayout afterHero={<Content />} />;
}

export { HomeLayout };
export * from '@rspress/core/theme-original';
