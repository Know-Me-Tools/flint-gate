import React from 'react';
import ComponentCreator from '@docusaurus/ComponentCreator';

export default [
  {
    path: '/flint-gate/docs',
    component: ComponentCreator('/flint-gate/docs', '289'),
    routes: [
      {
        path: '/flint-gate/docs',
        component: ComponentCreator('/flint-gate/docs', '9f2'),
        routes: [
          {
            path: '/flint-gate/docs',
            component: ComponentCreator('/flint-gate/docs', 'c20'),
            routes: [
              {
                path: '/flint-gate/docs/admin-api',
                component: ComponentCreator('/flint-gate/docs/admin-api', 'c87'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/cedar-policies',
                component: ComponentCreator('/flint-gate/docs/cedar-policies', '142'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/configuration',
                component: ComponentCreator('/flint-gate/docs/configuration', '5ce'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/forge-keys',
                component: ComponentCreator('/flint-gate/docs/forge-keys', '99a'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/getting-started',
                component: ComponentCreator('/flint-gate/docs/getting-started', 'e5a'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/intro',
                component: ComponentCreator('/flint-gate/docs/intro', 'a31'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/metrics',
                component: ComponentCreator('/flint-gate/docs/metrics', '35b'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/operations',
                component: ComponentCreator('/flint-gate/docs/operations', '1fc'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/sdks/',
                component: ComponentCreator('/flint-gate/docs/sdks/', '664'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/sdks/flutter',
                component: ComponentCreator('/flint-gate/docs/sdks/flutter', '91f'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/sdks/go',
                component: ComponentCreator('/flint-gate/docs/sdks/go', '563'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/sdks/rust',
                component: ComponentCreator('/flint-gate/docs/sdks/rust', '84c'),
                exact: true,
                sidebar: "docsSidebar"
              },
              {
                path: '/flint-gate/docs/sdks/typescript',
                component: ComponentCreator('/flint-gate/docs/sdks/typescript', '8d5'),
                exact: true,
                sidebar: "docsSidebar"
              }
            ]
          }
        ]
      }
    ]
  },
  {
    path: '/flint-gate/',
    component: ComponentCreator('/flint-gate/', 'eca'),
    exact: true
  },
  {
    path: '*',
    component: ComponentCreator('*'),
  },
];
