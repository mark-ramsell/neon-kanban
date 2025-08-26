import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App.tsx';
import './styles/index.css';
import { ClickToComponent } from 'click-to-react-component';
import * as Sentry from '@sentry/react';
// Install VS Code iframe keyboard bridge when running inside an iframe
import './vscode/bridge';

import {
  useLocation,
  useNavigationType,
  createRoutesFromChildren,
  matchRoutes,
} from 'react-router-dom';

Sentry.init({
  dsn: 'https://1065a1d276a581316999a07d5dffee26@o4509603705192449.ingest.de.sentry.io/4509605576441937',
  tracesSampleRate: 1.0,
  environment: import.meta.env.MODE === 'development' ? 'dev' : 'production',
  integrations: [
    Sentry.reactRouterV6BrowserTracingIntegration({
      useEffect: React.useEffect,
      useLocation,
      useNavigationType,
      createRoutesFromChildren,
      matchRoutes,
    }),
  ],
});
Sentry.setTag('source', 'frontend');

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Sentry.ErrorBoundary
      fallback={(fallbackProps: { error?: unknown }) => (
        <div style={{ padding: 16 }}>
          <p style={{ fontWeight: 600, marginBottom: 8 }}>An error has occurred</p>
          {(() => {
            const err = fallbackProps?.error as { message?: string } | undefined;
            if (err?.message) {
              return (
                <p style={{ color: '#b91c1c', marginBottom: 12 }}>{err.message}</p>
              );
            }
            return null;
          })()}
          <button
            onClick={() => window.location.reload()}
            style={{
              padding: '6px 10px',
              border: '1px solid #ccc',
              borderRadius: 6,
              background: 'transparent',
              cursor: 'pointer',
            }}
          >
            Reload
          </button>
        </div>
      )}
    >
      <ClickToComponent />
      <App />
    </Sentry.ErrorBoundary>
  </React.StrictMode>
);
