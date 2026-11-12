/**
 * An in-memory localStorage for the tests. Node's own global localStorage (left undefined without
 * --localstorage-file) hides the one jsdom brings, so the specs that need storage install this one.
 */
export class MemoryStorage implements Storage {
  private items = new Map<string, string>()
  get length(): number {
    return this.items.size
  }
  clear(): void {
    this.items.clear()
  }
  getItem(key: string): string | null {
    return this.items.get(key) ?? null
  }
  key(index: number): string | null {
    return [...this.items.keys()][index] ?? null
  }
  removeItem(key: string): void {
    this.items.delete(key)
  }
  setItem(key: string, value: string): void {
    this.items.set(key, String(value))
  }
}
