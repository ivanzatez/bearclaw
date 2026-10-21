import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import App from '@/App'
import { installServiceWorker } from '@/lib/push'
import { installViewportHeight } from '@/lib/viewport'
import './index.css'

installViewportHeight()
installServiceWorker()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
