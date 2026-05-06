import { file } from 'bun';

// 1. Get the input file path from command line arguments
const filePath = process.argv[2];

if (!filePath) {
  console.error("❌ Please provide a JSON file path.");
  console.error("Usage: bun run parse_structures.ts <path-to-file.json>");
  process.exit(1);
}

try {
  // 2. Read and parse the JSON file
  const fileContent = await file(filePath).text();
  const rawData = JSON.parse(fileContent);

  // Handle both raw arrays and standard Chrome Trace format { traceEvents: [...] }
  const events: any[] = Array.isArray(rawData) ? rawData : (rawData.traceEvents || []);

  if (events.length === 0) {
    console.log("No events found in the file.");
    process.exit(0);
  }

  // 3. Data structures to hold our unique keys
  const structures = new Map<string, Set<string>>();
  const argStructures = new Map<string, Set<string>>();
  const count = new Map<string, number>();

  // 4. Iterate through all events to discover structures
  for (const event of events) {
    const name = event.name;

    // Skip events without a name
    if (!name || name.split(" ")[0] === "Total") continue;

    // Initialize Sets if this is the first time seeing this 'name'
    if (!structures.has(name)) {
      structures.set(name, new Set());
      argStructures.set(name, new Set());
      count.set(name, 1);
    } else {
      count.set(name, count.get(name)! + 1);
    }

    // // Add all top-level keys
    // const nameKeys = structures.get(name)!;
    // for (const key of Object.keys(event)) {
    //   nameKeys.add(key);
    // }

    // Add all keys inside the 'args' object (if it exists)
    if (event.args && typeof event.args === 'object') {
      const argKeys = argStructures.get(name)!;
      for (const argKey of Object.keys(event.args)) {
        argKeys.add(argKey);
      }
    }
  }

  // 5. Format the output cleanly
  const result: Record<string, { /* top_level_keys: string[]; */ args_keys: string[]; count: number }> = {};

  for (const [name, keys] of structures.entries()) {
    result[name] = {
      // top_level_keys: Array.from(keys).sort(),
      args_keys: Array.from(argStructures.get(name)!).sort(),
      count: count.get(name)!
    };
  }

  // Print the result
  console.log(JSON.stringify(result, null, 2));

} catch (error) {
  console.error("❌ Error processing file:", error);
}