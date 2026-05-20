# EXPECT:
# found 2
# sum 9
# 0

def main() Int {
	values = [1, 2, 3]
	items = for item <- values yield {
		item + 1
	}
	unwrap first <- items.get(0) else return 0
	OS.println("found " + first)

	var total Int = 0
	for item <- items {
		total += item
	}
	OS.println("sum " + total)
	0
}
