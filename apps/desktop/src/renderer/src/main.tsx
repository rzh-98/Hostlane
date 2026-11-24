import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App } from './app/App'
import { APP_NAME } from './app/config'
import { initAppearance } from './theme/appearance'
import './styles/index.css'

document.title = APP_NAME
initAppearance()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>
)
