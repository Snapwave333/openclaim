import { ChangeDetectionStrategy, Component, inject, output } from '@angular/core';
import { PeopleService } from './people.service';

@Component({
  selector: 'oc-person-select',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="field field--inline">
      <label for="person-select">Person</label>
      <select id="person-select" [value]="people.selectedId() ?? ''" (change)="pick($any($event.target).value)" style="width: auto; min-width: 14rem">
        @if (people.people().length === 0) {
          <option value="">No profiles yet</option>
        }
        @for (p of people.people(); track p.id) {
          <option [value]="p.id">{{ p.display_name }}</option>
        }
      </select>
    </div>
  `,
})
export class PersonSelectComponent {
  readonly people = inject(PeopleService);
  readonly changed = output<string | null>();

  pick(id: string): void {
    this.people.select(id || null);
    this.changed.emit(id || null);
  }
}
