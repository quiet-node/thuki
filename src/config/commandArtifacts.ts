import { COMMANDS, type Command } from './commands';

const GENERATED_DOCS_NOTICE =
  '<!-- Generated from src/config/commands.ts by `bun run generate:commands`. Do not edit manually. -->';
const SLASH_COMMAND_LIST = COMMANDS.map((command) => command.trigger).join(
  ', ',
);

function renderDocsSection(command: Command): string {
  const sections = [
    `## ${command.trigger}`,
    '',
    command.docs.summary,
    '',
    `**Usage:** \`${command.docs.usage}\``,
    '',
    '**Examples:**',
    ...command.docs.examples.map((example) => `- ${example}`),
    '',
    `**Behavior:** ${command.docs.behavior}`,
  ];

  if (command.docs.composability) {
    sections.push('', `**Composable:** ${command.docs.composability}`);
  }

  if (command.docs.limit) {
    sections.push('', `**Limit:** ${command.docs.limit}`);
  }

  if (command.docs.permission) {
    sections.push('', `**Permission:** ${command.docs.permission}`);
  }

  if (command.docs.languageFormat) {
    sections.push('', `**Language format:** ${command.docs.languageFormat}`);
  }

  if (command.docs.defaultBehavior) {
    sections.push('', `**Default behavior:** ${command.docs.defaultBehavior}`);
  }

  return sections.join('\n');
}

function renderPromptSection(command: Command): string {
  return `${command.trigger}: ${command.promptHelp.summary}`;
}

export function renderCommandsMarkdown(): string {
  return [
    GENERATED_DOCS_NOTICE,
    '',
    '# Commands',
    '',
    'Commands are written as whole-word `/` triggers anywhere in your message. Press `/` to open the command suggestion menu, then Tab to complete or Enter to select.',
    '',
    'Commands can be combined when their behavior allows it. For example, `/screen /think` captures the screen and enables extended reasoning, while `/think /tldr` summarizes with thinking enabled.',
    '',
    'Commands that operate on text follow a consistent input priority:',
    '',
    '1. **Highlighted text + no typed text:** highlighted text is the input',
    '2. **No highlighted text + typed text after command:** typed text is the input',
    '3. **Both present:** highlighted text is the primary input; typed text is appended as an additional instruction',
    '',
    'This means you can highlight text anywhere on screen, summon Thuki with double-tap Control, type a command, and hit Enter without retyping the selected content.',
    '',
    ...COMMANDS.flatMap((command, index) => {
      const section = renderDocsSection(command);
      return index === COMMANDS.length - 1
        ? [section]
        : [section, '', '---', ''];
    }),
    '',
  ].join('\n');
}

export function renderSlashCommandPromptAppendix(): string {
  return [
    '# Supported slash commands',
    '',
    `These are Thuki's only built-in slash commands: ${SLASH_COMMAND_LIST}.`,
    '',
    'If the user asks what slash commands are available, what built-in commands exist, or how to use them, answer with the slash-command list below. Do not answer about generic tools, tool availability, or function calling.',
    '',
    ...COMMANDS.flatMap((command, index) => {
      const section = renderPromptSection(command);
      return index === COMMANDS.length - 1 ? [section] : [section, ''];
    }),
    '',
  ].join('\n');
}
