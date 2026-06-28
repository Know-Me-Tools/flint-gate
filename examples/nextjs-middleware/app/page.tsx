export default function Home() {
  return (
    <main style={{ padding: "2rem", fontFamily: "system-ui" }}>
      <h1>Flint Gate + Next.js Middleware</h1>
      <p>
        Public page. Try visiting{" "}
        <code>/api/hello</code> or any{" "}
        <code>/protected/...</code> route through Flint Gate.
      </p>
      <p>
        Requests without the <code>x-flint-authenticated</code> header or
        missing the <code>chat</code> scope will be rejected with 401/403.
      </p>
    </main>
  );
}
