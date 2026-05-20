# EXPECT:
# then
# else-if
# picked 5
# picked2 7
# picked3 9
# binding 7
# 0

def main() Int {
    if true then OS.println("then") else OS.println("else")
    if false then OS.println("nope") else if true then OS.println("else-if") else OS.println("also nope")

    picked = if false then 1 else 5
    OS.println("picked " + picked)
    picked2 = if true then 7
    else 8
    OS.println("picked2 " + picked2)
    picked3 = if false then 1
    else if false then 2
    else 9
    OS.println("picked3 " + picked3)

    values = [7]
    if value <- values.get(0) then OS.println("binding " + value) else OS.println("binding none")

    0
}
