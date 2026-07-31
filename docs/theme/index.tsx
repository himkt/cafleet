import { useFrontmatter } from '@rspress/core/runtime';
import {
  HomeBackground,
  HomeFeature,
  HomeFooter,
  HomeHero,
} from '@rspress/core/theme-original';

const demoVideo = (
  <iframe
    className="cafleet-home-demo"
    src="https://www.youtube.com/embed/cLLp-eoWFBg"
    title="CAFleet demo video"
    allowFullScreen
  />
);

// Mirrors the default HomeLayout's markdown branch (not exported by the theme)
// so the llms .md render of the home page keeps the hero summary.
function HomeLayoutMarkdown() {
  const { frontmatter } = useFrontmatter();
  const hero = frontmatter?.hero;
  const lines: string[] = [];
  if (hero?.name) {
    lines.push(`# ${hero.name}`, '');
  }
  if (hero?.text) {
    lines.push(hero.text, '');
  }
  if (hero?.tagline) {
    lines.push(`> ${hero.tagline}`, '');
  }
  if (hero?.actions?.length) {
    lines.push(
      hero.actions.map((action) => `[${action.text}](${action.link})`).join(' | '),
      '',
    );
  }
  return <>{lines.join('\n')}</>;
}

function HomeLayout() {
  if (import.meta.env.SSG_MD) {
    return <HomeLayoutMarkdown />;
  }
  return (
    <>
      <HomeBackground />
      <HomeHero image={demoVideo} />
      <HomeFeature />
      <HomeFooter />
    </>
  );
}

export { HomeLayout };
export * from '@rspress/core/theme-original';
