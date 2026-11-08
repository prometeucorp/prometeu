/** Isolated proposal: render the real sidebar, chat blocks and composer with sample data. */
import './src/style.css';
import './workspace-hierarchy-preview.css';
import * as sidebar from './src/sidebar';
import { button } from './src/ui';
import { brand, icon } from './src/icons';
import { use, t } from './src/i18n';
import { h, template } from './src/util';
import { composer } from './src/components/chat/composer';
import { conversationBlock } from './src/components/chat/blocks';
import { renderUserMessage } from './src/components/chat/content';
import type { Board, Workspace } from './src/types';

const english = new URLSearchParams(location.search).get('lang') === 'en';
use(english ? 'en' : 'pt-BR');
const labels = english ? {
  prototype: 'Visual proposal · sample conversation', children: '7 sub-workspaces',
  responsible: 'Responsible agent', parent: 'Main workspace', simulated: 'Preview only. No message is sent.',
  rule: 'One main workspace and seven sub-workspaces. Each sub-workspace has one conversation and cannot contain other sub-workspaces.',
} : {
  prototype: 'Proposta visual · conversa ilustrativa', children: '7 sub-workspaces',
  responsible: 'Agente responsável', parent: 'Workspace principal', simulated: 'Prévia visual. Nenhuma mensagem é enviada.',
  rule: 'Um workspace principal e sete sub-workspaces. Cada sub-workspace tem uma única conversa e não pode conter outros sub-workspaces.',
};
const samples = [
  ['Planejar melhorias no projeto', 'prometeu/nevoeiro-jovem-2093'],
  ['Múltiplos Comprovantes e Contratos', 'delegated-7c0c42f8-606'],
  ['Status e datas nos pagamentos', 'delegated-36aa5b92-97c'],
  ['Adicionar Instagram ao Supplier', 'delegated-32f1ec86-670'],
  ['Descoberta do Orçamento do Evento', 'delegated-95de6cac-dba'],
  ['Calendário mensal de contratações', 'delegated-7fbf4f28-623'],
  ['Descoberta do portal do cliente', 'delegated-473f065b-7f0'],
  ['Lembretes de Pagamentos e Créditos', 'delegated-6d1a4a4e-137'],
];
const workspaces: Workspace[] = samples.map(([title, branch], index) => ({
  id: `preview-${index}`, title, branch, project: 'preview-sil', repo: '/sample/sil', repo_name: 'sil',
  worktree: `/sample/worktrees/${branch}`, repos: [{ path: '/sample/sil', name: 'sil', worktree: `/sample/worktrees/${branch}`, base: 'main', pr: null }],
  stage: 'Fazendo', archived: false, pinned: false, unread: false, agent: 'codex',
  model: index === 0 ? 'gpt-6-astra' : '', effort: 'high', mcp: null, plugins: null, skills: null,
  port: null, issue: null, cleaned: false, shared: false, audience: null, remote_control: false,
  confirm_messages: true, preparing: false, failed: null,
  tabs: [{ id: `tab-${index}`, title: index === 0 ? 'GPT-6-Astra' : 'Codex', status: 'pronta', note: null, tokens: null }], active: `tab-${index}`,
}));
const [parent, ...children] = workspaces;
const board: Board = { stages: ['Preparando', 'Fazendo', 'Code review', 'Feito'], projects: [{ id: 'preview-sil', name: 'sil', path: '/sample/sil' }], workspaces };
const $ = (id: string) => document.getElementById(id)!;
const noop = () => {};
let selected = children[0];
let collapsed = false;
const hooks: sidebar.Hooks = {
  open: workspace => select(workspace), activeTab: () => selected.active,
  setStage: noop, rename: noop, archive: noop, drop: noop, finish: noop, cleanup: noop, pin: noop, unread: noop,
  reveal: noop, copyPath: noop, toDesk: noop, toIssues: noop, toArchived: noop,
  toNotifications: noop, notifications: () => 0, issues: () => null,
  addProject: noop, removeProject: noop, reorderProjects: noop, groupProjects: noop,
  projectTools: noop, openProject: noop, newWorkspace: noop,
  terminals: { available: () => false, live: () => false, open: noop, create: noop, createIn: noop, rename: noop, close: noop },
};

function renderRail() {
  sidebar.setOpen(selected.id);
  sidebar.render(board, hooks);
  // Reuse production rows; sub-workspaces expose their single conversation directly.
  const parentCard = document.querySelector<HTMLElement>(`[data-workspace="${parent.id}"]`)!;
  const group = h('div', 'preview-family');
  parentCard.before(group);
  group.append(parentCard);
  const childList = h('div', 'preview-children');
  childList.id = 'preview-children'; childList.hidden = collapsed;
  const toggle = button('', () => {
    collapsed = !collapsed; childList.hidden = collapsed;
    toggle.setAttribute('aria-expanded', String(!collapsed));
    toggle.innerHTML = `${icon(collapsed ? 'chevron-right' : 'chevron-down', 12)}<span>${labels.children}</span>`;
  }, 'ghost');
  toggle.classList.add('preview-family-toggle'); toggle.title = labels.rule;
  toggle.setAttribute('aria-expanded', String(!collapsed)); toggle.setAttribute('aria-controls', childList.id);
  toggle.innerHTML = `${icon(collapsed ? 'chevron-right' : 'chevron-down', 12)}<span>${labels.children}</span>`;
  group.append(toggle, childList);
  children.forEach(child => {
    const card = document.querySelector<HTMLElement>(`[data-workspace="${child.id}"]`)!;
    card.querySelector('.railagents')?.remove();
    card.querySelector('.wsbranch')?.remove();
    const control = card.querySelector<HTMLButtonElement>('.navitem')!;
    control.querySelector('.wsidentity')!.before(template('span', 'preview-provider', brand(child.agent, 15)));
    control.title = `${child.title} · ${child.tabs[0].title}\n${child.branch}`;
    control.setAttribute('aria-label', `${child.title} · ${child.tabs[0].title}`);
    childList.append(card);
  });
  // Disable unrelated application effects in this isolated proposal, including account login.
  $('railbody').querySelectorAll<HTMLButtonElement>('button').forEach(control => {
    if (!control.closest('.railworkspace') && control !== toggle) {
      control.addEventListener('click', event => { event.preventDefault(); event.stopImmediatePropagation(); }, true);
    }
  });
}

function select(workspace: Workspace) {
  selected = workspace;
  renderRail();
  $('crumb').replaceChildren(h('span', '', 'sil'), h('span', 'sep', '/'), h('span', '', workspace.title));
  const tab = template('div', 'tab on', `${brand('codex', 15)}<span>${workspace.tabs[0].title}</span>`);
  $('tabbar').replaceChildren(tab);
  $('tabbar').hidden = workspace !== parent;
  const family = $('family-context');
  const parentButton = button(parent.title, () => select(parent), 'ghost');
  parentButton.classList.add('preview-parent-link'); parentButton.title = parent.branch;
  family.replaceChildren();
  if (workspace !== parent) {
    family.append(template('span', 'preview-family-icon', icon('git-branch', 13)), h('span', '', labels.parent), parentButton);
  } else {
    family.append(template('span', 'preview-family-icon', icon('git-branch', 13)), h('span', '', labels.children));
  }
  family.append(h('span', 'spacer'), h('span', 'preview-owner', `${labels.responsible} · GPT-6-Astra`));
  const feed = $('feed'); feed.replaceChildren();
  const user = template('div', 'turn user', '<div class="bubble"></div>');
  renderUserMessage(user.firstElementChild as HTMLElement, workspace === parent
    ? 'Planeje as melhorias do projeto e divida os trabalhos em workspaces separados para executar em paralelo.'
    : `Implemente a melhoria: ${workspace.title}. Trabalhe neste worktree e reporte o resultado ao agente responsável.`);
  feed.append(user);
  const turn = h('div', 'turn bot');
  turn.append(conversationBlock({ kind: 'text', text: workspace === parent
    ? 'Dividi o trabalho em **7 workspaces**, cada um com seu próprio worktree e uma conversa Codex.\n\nVou acompanhar os resultados por aqui. Os trabalhos delegados aparecem agrupados abaixo deste workspace na barra lateral.'
    : `Este workspace cuida de **${workspace.title}**.\n\nO planejamento continua em **${parent.title}**, com GPT-6-Astra como agente responsável.\n\nAs alterações deste trabalho ficam no worktree \`${workspace.branch}\`.`, }, false));
  feed.append(turn);
  if (workspace === parent) {
    const links = h('div', 'preview-delegations');
    children.forEach(child => {
      const link = button(child.title, () => select(child), 'ghost');
      link.classList.add('preview-delegation');
      link.prepend(template('span', '', icon('git-branch', 13)));
      links.append(link);
    });
    feed.append(links);
  }
  feed.append(h('div', 'sys', labels.prototype));
  $('chatwrap').querySelector('.composer')?.remove();
  const input = composer({ send: noop, stop: noop, addFile: noop, voice: noop, quote: noop, actions: noop, voiceAvailable: false });
  input.area.placeholder = t('chat.input.hint');
  input.area.readOnly = true; input.area.title = labels.simulated;
  input.send.disabled = true; input.withModel.hidden = false;
  input.model.textContent = workspace === parent ? 'GPT-6-Astra' : 'Codex';
  input.effort.querySelector('.el')!.textContent = 'High';
  input.hint.textContent = labels.simulated;
  $('chatwrap').append(input.root);
}
$('prototype-label').textContent = labels.prototype;
$('settings-icon').innerHTML = icon('settings', 14);
$('railbody').addEventListener('click', event => {
  if ((event.target as HTMLElement).closest('.group')) { event.preventDefault(); event.stopImmediatePropagation(); }
}, true);
$('railbody').addEventListener('contextmenu', event => { event.preventDefault(); event.stopImmediatePropagation(); }, true);
select(selected);
