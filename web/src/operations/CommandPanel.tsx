import { useState, type FormEvent } from 'react';
import type { Command, CommandParameter, NodeOperations } from './data';

export function CommandPanel({ controls }: { controls: NodeOperations }) {
  const [name, setName] = useState('');
  const selected = controls.catalog.commands.find((command) => command.name === name);

  return (
    <section className="monitor-view" aria-label="预定义命令">
      <div className="section-bar"><h2>预定义命令</h2></div>
      <label className="search-field">命令
        <select value={name} onChange={(event) => setName(event.target.value)}>
          <option value="">选择命令</option>
          {controls.catalog.commands.map((command) => <option key={command.name} value={command.name}>{command.name}</option>)}
        </select>
      </label>
      {selected && <Parameters key={selected.name} command={selected} controls={controls} />}
    </section>
  );
}

function Parameters({ command, controls }: { command: Command; controls: NodeOperations }) {
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    controls.propose({
      type: 'command',
      name: command.name,
      parameters: Object.fromEntries(new FormData(event.currentTarget)) as Record<string, string>,
    });
  }

  return (
    <form className="fields detail-panel" onSubmit={submit}>
      <p className="observation">超时 · {command.timeout_seconds} 秒</p>
      {command.parameters.length > 0 && (
        <div className="form-grid">{command.parameters.map((parameter) => <Field key={parameter.name} parameter={parameter} />)}</div>
      )}
      <div className="dialog-actions"><button type="submit" className="primary">检查操作</button></div>
    </form>
  );
}

function Field({ parameter }: { parameter: CommandParameter }) {
  return (
    <label>{parameter.name}
      {parameter.type === 'choice' && (
        <select name={parameter.name} required>
          <option value="">选择值</option>
          {parameter.values.map((value) => <option value={value} key={value}>{value}</option>)}
        </select>
      )}
      {parameter.type === 'integer' && <input name={parameter.name} type="number" min={parameter.min} max={parameter.max} step={1} required />}
      {parameter.type === 'identifier' && <input name={parameter.name} maxLength={parameter.max_length} required />}
    </label>
  );
}
