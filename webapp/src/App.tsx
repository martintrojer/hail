const appVersion = import.meta.env.VITE_APP_VERSION ?? '0.0.0';

export default function App() {
  return (
    <main className="min-h-screen bg-slate-950 text-slate-50">
      <section className="mx-auto flex min-h-screen max-w-5xl flex-col items-center justify-center gap-4 px-6 text-center">
        <p className="text-sm font-medium uppercase tracking-[0.35em] text-sky-300">
          hail
        </p>
        <h1 className="text-5xl font-semibold tracking-tight">hail</h1>
        <p className="max-w-xl text-balance text-slate-300">
          React SPA scaffold for the self-hostable hey-style email client.
        </p>
        <p className="text-sm text-slate-500">version {appVersion}</p>
      </section>
    </main>
  );
}
