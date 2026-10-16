console.log("sync-before");
import("./two.cjs").then(function(two) {
	console.log("default:" + two.default);
}).catch(function(error) {
	queueMicrotask(() => { throw error; });
});
console.log("sync-after");
