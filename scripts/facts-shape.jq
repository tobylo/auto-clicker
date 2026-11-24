# Checks docs/facts.json's shape and prints one line per problem; prints
# nothing when the file has the one shape it may have. The project page's
# build checks the same shape (tobylo/auto-clicker-site, src/data/program.ts).

# A non-empty string on one line, with no control characters.
def text: type == "string" and length > 0 and (test("[\\x00-\\x1f\\x7f]") | not);

# An object with exactly the keys in $required, and any of $optional.
def shaped($required; $optional):
  type == "object" and (($required - keys) == []) and ((keys - $required - $optional) == []);

# A file in this repository, as a plain relative path.
def repo_path:
  text and test("^[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*$")
  and (split("/") | all(. != "." and . != ".."));

def slug: text and test("^[a-z0-9]+(-[a-z0-9]+)*$");

def check(ok; $problem): if ok then empty else $problem end;

def unique_by_key($key): (map(.[$key]?) | length) == (map(.[$key]?) | unique | length);

def line_problems($where):
  if shaped(["command"]; ["guideOnly"]) then
    check(.command | text; "\($where): command must be one line of text"),
    check((has("guideOnly") | not) or .guideOnly == true; "\($where): guideOnly may only be true")
  elif shaped(["remark"]; []) then
    check(.remark | text; "\($where): remark must be one line of text")
  else
    "\($where): a line is either {command, guideOnly?} or {remark}"
  end;

def note_problems($where):
  if shaped(["text", "link", "linkText"]; []) then
    check((.text | text) and (.linkText | text); "\($where): note text and linkText must be text"),
    check(.link | repo_path; "\($where): note link must be a path in this repository")
  else
    "\($where): note must have exactly text, link and linkText"
  end;

def available_problems($where):
  check(.readme | text; "\($where): an available platform names its README heading (readme)"),
  check([.lines[]? | select(has("command"))] | length > 0; "\($where): an available platform has commands");

def soon_problems($where):
  check(has("readme") | not; "\($where): a platform that is soon has no README heading"),
  check(.lines == []; "\($where): a platform that is soon has no install lines");

def platform_problems:
  .where as $where | .value |
  if shaped(["id", "label", "system", "status", "scope", "lines"]; ["readme", "note"]) then
    check(.id | slug; "\($where): id must be lowercase words joined by -"),
    check((.label | text) and (.system | text) and (.scope | text); "\($where): label, system and scope must be text"),
    (if .lines | type == "array" then
       (.lines | to_entries[] | .key as $i | .value | line_problems("\($where) line \($i + 1)"))
     else "\($where): lines must be a list" end),
    (if has("note") then .note | note_problems($where) else empty end),
    (if .status == "available" then available_problems($where)
     elif .status == "soon" then soon_problems($where)
     else "\($where): status must be available or soon" end)
  else
    "\($where): a platform has id, label, system, status, scope, lines, and may have readme and note"
  end;

def binding_problems($where):
  if shaped(["key", "name", "does"]; []) then
    check((.key | text) and (.name | text) and (.does | text); "\($where): key, name and does must be text")
  else
    "\($where): a hotkey has exactly key, name and does"
  end;

def hotkeys_problems:
  if shaped(["modifiers", "bindings"]; []) then
    check(.modifiers | text; "hotkeys: modifiers must be text"),
    (if (.bindings | type) == "array" and (.bindings | length) > 0 then
       (.bindings | to_entries[] | .key as $i | .value | binding_problems("hotkey \($i + 1)")),
       check(.bindings | unique_by_key("key"); "hotkeys: each key appears once")
     else "hotkeys: bindings must be a non-empty list" end)
  else
    "hotkeys: has exactly modifiers and bindings"
  end;

def source_problems($where):
  if shaped(["path", "quote"]; []) then
    check(.path | repo_path; "\($where): source path must be a path in this repository"),
    check(.quote | text; "\($where): source quote must be one line of text")
  else
    "\($where): source has exactly path and quote"
  end;

def figure_problems:
  .where as $where | .value |
  if shaped(["id", "value", "unit", "label", "basis", "detail", "source"]; []) then
    check(.id | slug; "\($where): id must be lowercase words joined by -"),
    check((.value | text) and (.label | text) and (.detail | text); "\($where): value, label and detail must be text"),
    check(.unit == "" or (.unit | text); "\($where): unit must be text, or empty"),
    check(.basis == "measured" or .basis == "rule"; "\($where): basis must be measured or rule"),
    (.source | source_problems($where))
  else
    "\($where): a figure has exactly id, value, unit, label, basis, detail and source"
  end;

def list_problems($name; f):
  if (.[$name] | type) == "array" and (.[$name] | length) > 0 then
    (.[$name] | to_entries[] | {where: "\($name) \(.key + 1)", value} | f),
    check(.[$name] | unique_by_key("id"); "\($name): each id appears once")
  else "\($name) must be a non-empty list" end;

if shaped(["version", "links", "platforms", "hotkeys", "figures"]; []) then
  check(.version == 1; "version must be 1"),
  (if (.links | type) == "object" and (.links | length) > 0 then
     (.links | to_entries[] |
       check(.key | slug; "links: \(.key | tojson) is not a name of lowercase words joined by -"),
       check(.value | repo_path; "links.\(.key): must be a path in this repository"))
   else "links must be a non-empty object" end),
  list_problems("platforms"; platform_problems),
  check([.platforms[]? | .readme? // empty] | (length == (unique | length)); "platforms: each README heading belongs to one platform"),
  (.hotkeys | hotkeys_problems),
  list_problems("figures"; figure_problems)
else
  "the file is an object with exactly version, links, platforms, hotkeys and figures"
end
