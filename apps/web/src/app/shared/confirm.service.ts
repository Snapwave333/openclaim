import { Dialog, DIALOG_DATA, DialogRef } from '@angular/cdk/dialog';
import { ChangeDetectionStrategy, Component, Injectable, inject, signal } from '@angular/core';
import { firstValueFrom } from 'rxjs';

export interface ConfirmOptions {
  title: string;
  message: string;
  confirmLabel?: string;
  danger?: boolean;
  /** When set, the user must type this exact text to confirm. */
  typeToConfirm?: string;
}

@Component({
  selector: 'oc-confirm-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="dialog" role="document">
      <h2 id="confirm-title">{{ data.title }}</h2>
      <p id="confirm-desc">{{ data.message }}</p>
      @if (data.typeToConfirm) {
        <div class="field">
          <label for="confirm-typed">Type <code>{{ data.typeToConfirm }}</code> to continue</label>
          <input id="confirm-typed" type="text" autocomplete="off" [value]="typed()" (input)="typed.set($any($event.target).value)" />
        </div>
      }
      <div class="dialog__actions">
        <button class="btn" type="button" (click)="ref.close(false)">Cancel</button>
        <button class="btn" [class.btn--danger]="data.danger" [class.btn--primary]="!data.danger" type="button" (click)="ref.close(true)" [disabled]="!!data.typeToConfirm && typed() !== data.typeToConfirm" data-action="confirm">
          {{ data.confirmLabel ?? 'Confirm' }}
        </button>
      </div>
    </div>
  `,
})
export class ConfirmDialogComponent {
  readonly data = inject<ConfirmOptions>(DIALOG_DATA);
  readonly ref = inject<DialogRef<boolean>>(DialogRef);
  readonly typed = signal('');
}

/** Accessible confirmation dialogs via Angular CDK (focus trap, Escape to close, aria attributes). */
@Injectable({ providedIn: 'root' })
export class ConfirmService {
  private readonly dialog = inject(Dialog);

  async ask(opts: ConfirmOptions): Promise<boolean> {
    const ref = this.dialog.open<boolean, ConfirmOptions>(ConfirmDialogComponent, {
      data: opts,
      ariaLabelledBy: 'confirm-title',
      ariaDescribedBy: 'confirm-desc',
      hasBackdrop: true,
      disableClose: false,
    });
    const result = await firstValueFrom(ref.closed);
    return result === true;
  }
}
