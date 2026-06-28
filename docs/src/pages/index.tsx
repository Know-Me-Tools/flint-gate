import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';

export default function Home(): JSX.Element {
  const {siteConfig} = useDocusaurusContext();

  return (
    <Layout title={siteConfig.title} description="Flint Gate documentation">
      <main className="container margin-vert--xl">
        <div className="text--center">
          <Heading as="h1">{siteConfig.title}</Heading>
          <p className="hero__subtitle">{siteConfig.tagline}</p>
          <div className="margin-top--lg">
            <Link className="button button--primary button--lg" to="/docs/intro">
              Read the docs
            </Link>
          </div>
        </div>

        <div className="row margin-top--xl">
          <div className="col col--4">
            <Heading as="h3">Auth proxy</Heading>
            <p>
              Authenticate requests with Ory Kratos sessions, JWTs, or API keys
              before forwarding them to upstream services.
            </p>
          </div>
          <div className="col col--4">
            <Heading as="h3">Streaming</Heading>
            <p>
              Pass through SSE, WebSocket, and NDJSON streams without buffering
              full responses. Count tokens from AG-UI events mid-stream.
            </p>
          </div>
          <div className="col col--4">
            <Heading as="h3">Runtime routing</Heading>
            <p>
              Manage routes from YAML or Postgres via the admin API, with
              hot-reload across all running instances.
            </p>
          </div>
        </div>
      </main>
    </Layout>
  );
}
