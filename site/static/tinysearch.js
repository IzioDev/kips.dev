class TinysearchIndex {
  constructor(instance) {
    this.instance = instance;
    this.memory = instance.exports.memory;
    this.searchFunction = instance.exports.search;
    this.freeSearchResult = instance.exports.free_search_result;
  }

  writeQuery(query) {
    const bytes = new TextEncoder().encode(`${query}\0`);
    const pageCount = Math.max(1, Math.ceil(bytes.length / 65536));
    const previousPageCount = this.memory.grow(pageCount);
    const pointer = previousPageCount * 65536;
    new Uint8Array(this.memory.buffer, pointer, bytes.length).set(bytes);
    return pointer;
  }

  readResult(pointer) {
    const memory = new Uint8Array(this.memory.buffer);
    let end = pointer;
    while (end < memory.length && memory[end] !== 0) end += 1;
    return new TextDecoder().decode(memory.subarray(pointer, end));
  }

  search(query, resultCount = 8) {
    const resultPointer = this.searchFunction(this.writeQuery(query), resultCount);
    if (!resultPointer) return [];
    const result = this.readResult(resultPointer);
    this.freeSearchResult(resultPointer);
    return JSON.parse(result);
  }
}

export async function initTinysearch() {
  const wasmUrl = new URL("tinysearch/tinysearch_engine.wasm", import.meta.url);
  const response = await fetch(wasmUrl);
  if (!response.ok) throw new Error("Tinysearch index unavailable");

  if (WebAssembly.instantiateStreaming) {
    try {
      const module = await WebAssembly.instantiateStreaming(response.clone());
      return new TinysearchIndex(module.instance);
    } catch (_) {}
  }

  const module = await WebAssembly.instantiate(await response.arrayBuffer());
  return new TinysearchIndex(module.instance);
}
