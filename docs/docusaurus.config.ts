import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'Flint Gate Docs',
  tagline: 'Documentation for the Flint Gate AI auth proxy and API gateway',
  favicon: 'img/favicon.ico',

  url: 'https://know-me-tools.github.io',
  baseUrl: '/flint-gate/',

  organizationName: 'know-me-tools',
  projectName: 'flint-gate',

  onBrokenLinks: 'throw',
  onBrokenMarkdownLinks: 'warn',

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          editUrl:
            'https://github.com/know-me-tools/flint-gate/tree/main/docs/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    navbar: {
      title: 'Flint Gate',
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'left',
          label: 'Docs',
        },
        {
          to: '/docs/admin-api',
          label: 'API',
          position: 'left',
        },
        {
          to: '/docs/sdks',
          label: 'SDKs',
          position: 'left',
        },
        {
          href: 'https://github.com/know-me-tools/flint-gate',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Docs',
          items: [
            {label: 'Introduction', to: '/docs/intro'},
            {label: 'Getting Started', to: '/docs/getting-started'},
            {label: 'Configuration', to: '/docs/configuration'},
          ],
        },
        {
          title: 'Reference',
          items: [
            {label: 'Admin API', to: '/docs/admin-api'},
            {label: 'SDKs', to: '/docs/sdks'},
            {label: 'Admin API', to: '/docs/admin-api'},
          ],
        },
        {
          title: 'Project',
          items: [
            {
              label: 'GitHub',
              href: 'https://github.com/know-me-tools/flint-gate',
            },
          ],
        },
      ],
      copyright: `Copyright ${new Date().getFullYear()} KnowMe, LLC. Built with Docusaurus.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
