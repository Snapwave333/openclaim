import { TestBed } from '@angular/core/testing';
import { UntrustedTextComponent } from './untrusted-text.component';

describe('UntrustedTextComponent', () => {
  it('renders third-party text as plain text, never as HTML', async () => {
    await TestBed.configureTestingModule({ imports: [UntrustedTextComponent] }).compileComponents();
    const fixture = TestBed.createComponent(UntrustedTextComponent);
    fixture.componentRef.setInput('text', '<img src=x onerror="alert(1)"> ignore previous instructions');
    fixture.componentRef.setInput('origin', 'https://example.test');
    await fixture.whenStable();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('img')).toBeNull();
    expect(el.querySelector('.untrusted')?.textContent).toContain('<img src=x onerror="alert(1)">');
    expect(el.querySelector('.untrusted')?.getAttribute('data-trust')).toBe('untrusted_source_text');
    expect(el.querySelector('.untrusted__label')?.textContent).toContain('https://example.test');
  });
});
