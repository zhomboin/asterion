import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './styles/tokens.css';
import './styles/base.css';
import './styles/components.css';
import './styles/layout.css';
import './styles/pages.css';

const root = document.getElementById('root');
if (!root) {
  throw new Error('找不到 Web 应用挂载点');
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
