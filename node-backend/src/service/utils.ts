export class Utils {
  static lowerCaseKeys<T extends object>(obj: T): Record<string, T[keyof T]> {
    return Object.fromEntries(
      Object.entries(obj).map(([k, v]) => [k.toLowerCase(), v])
    ) as Record<string, T[keyof T]>;
  }
}
