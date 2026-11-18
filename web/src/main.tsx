import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './styles.css';
import './monitor.css';
import './operations.css';
import './terminal.css';

const root = document.getElementById('root');
if (!root) {
  throw new Error('找不到 Web 应用挂载点');
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
