import { serviceVerbs, serviceVerbLabel, type ServiceVerb } from './data';

export function ServiceActions({ title, onChoose }: { title: string; onChoose: (verb: ServiceVerb) => void }) {
  return (
    <div className="actions service-actions" aria-label={title}>
      {serviceVerbs.map((verb) => (
        <button type="button" key={verb} className={verb === 'stop' ? 'danger' : undefined} onClick={() => onChoose(verb)}>
          {serviceVerbLabel[verb]}
        </button>
      ))}
    </div>
  );
}
