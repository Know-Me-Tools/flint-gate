import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'intro',
    'getting-started',
    'configuration',
    'admin-api',
    {
      type: 'category',
      label: 'SDKs',
      link: {type: 'doc', id: 'sdks/index'},
      items: [
        'sdks/rust',
        'sdks/go',
        'sdks/typescript',
        'sdks/flutter',
      ],
    },
    {
      type: 'category',
      label: 'Operations',
      items: [
        'operations',
        'cedar-policies',
        'metrics',
      ],
    },
  ],
};

export default sidebars;
