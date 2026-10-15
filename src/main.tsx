import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { applyTheme, startupTheme } from './theme';
import './styles.css';
import './redesign.css';
import './polish.css';
applyTheme(startupTheme());
ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
