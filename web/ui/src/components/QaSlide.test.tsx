import { renderToStaticMarkup } from 'react-dom/server'
import { expect, test } from 'vitest'

import { parseSlide } from '@/lib/qa'
import { QaSlide } from './QaSlide'

test('a slide shows its title, points, numbered code with the highlighted line, and its diagram', () => {
  const slide = parseSlide(
    JSON.stringify({
      title: 'delegate_task',
      points: ['Glavni agent čeka rezultat'],
      code: { path: 'src/tools/delegate-task-tool.ts', start: 41, text: 'export const tool = {\n  name: "delegate_task",\n}', highlight: [2] },
      diagram: { nodes: [{ id: 'main', label: 'Main agent' }, { id: 'sub', label: 'Subagent', note: 'fast' }], edges: [['main', 'sub', 'delegate_task']] },
    }),
  )
  const html = renderToStaticMarkup(<QaSlide slide={slide} />)
  expect(html).toContain('delegate_task')
  expect(html).toContain('Glavni agent čeka rezultat')
  expect(html).toContain('src/tools/delegate-task-tool.ts')
  expect(html).toMatch(/>41<.*>42<.*>43</s)
  expect(html).toContain('border-primary bg-primary/12')
  expect(html).toContain('Main agent')
  expect(html).toContain('<path d="M')
  expect(renderToStaticMarkup(<QaSlide slide={null} />)).toContain('The first slide goes up')
})
