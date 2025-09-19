export class Utils {
  static lowerCaseKeys<T extends object>(obj: T): Record<string, T[keyof T]> {
    return Object.fromEntries(
      Object.entries(obj).map(([k, v]) => [k.toLowerCase(), v])
    ) as Record<string, T[keyof T]>;
  }
  
  /**
   * Returns a hash code from a string
   * @param  {String} str The string to hash.
   * @return {Number}    A 32bit integer
   * @see http://werxltd.com/wp/2010/05/13/javascript-implementation-of-javas-string-hashcode-method/
   */
  static hashCode(str: String) {
    let hash = 0;
    for (let i = 0, len = str.length; i < len; i++) {
      let chr = str.charCodeAt(i);
      hash = (hash << 5) - hash + chr;
      hash |= 0; // Convert to 32bit integer
    }
    return hash;
  }
}
