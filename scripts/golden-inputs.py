#!/usr/bin/env python3
"""Write the inputs of the golden cases in crates/wasm/tests/golden.

The cases are plain JSON files; this script is how their inputs were made, and
how to remake them when a type that crosses the wasm boundary gains a field
(update `issue()` / `project()` below, then):

    python3 scripts/golden-inputs.py phase1
    UPDATE_GOLDEN=1 cargo test -p linear-wasm --test golden   # fills in `expected`
    python3 scripts/golden-inputs.py phase2                    # diff cases, built from audit's answers
    UPDATE_GOLDEN=1 cargo test -p linear-wasm --test golden
    git diff crates/wasm/tests/golden                          # read it

A case that already has an `expected` keeps it until UPDATE_GOLDEN rewrites it.
"""
import copy, hashlib, hmac, json, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + '/'
OUT = ROOT + 'crates/wasm/tests/golden/'
FIX = ROOT + 'crates/core/tests/fixtures/'
NOW = '2026-10-20T12:00:00Z'


def write(kind, name, description, input_, expected=None):
    d = OUT + kind
    os.makedirs(d, exist_ok=True)
    path = '%s/%s.json' % (d, name)
    if os.path.exists(path) and expected is None:
        old = json.load(open(path))
        expected = old.get('expected')
    with open(path, 'w') as f:
        json.dump({'description': description, 'input': input_, 'expected': expected}, f, indent=2, ensure_ascii=False, sort_keys=True)
        f.write('\n')


# ------------------------------------------------------------ audit builders

def user(me):
    ident, name = ('u-me', 'Me') if me else ('u-other', 'Other')
    return {'id': ident, 'name': name, 'displayName': name.lower(), 'email': ident + '@example.com', 'active': True, 'isMe': me}


def state(kind):
    return {'id': 's-' + kind, 'name': 'State ' + kind, 'type': kind}


def project_ref(slug):
    return {'id': 'p-' + slug, 'slugId': slug, 'name': 'Project ' + slug, 'url': 'https://linear.app/x/project/' + slug}


def milestone(slug, name, target, status):
    return {'id': 'm-' + name, 'name': name, 'description': None, 'targetDate': target, 'status': status,
            'sortOrder': 0.0, 'progress': 0.0, 'project': project_ref(slug)}


def issue(identifier, **kw):
    i = {
        'id': 'i-' + identifier, 'identifier': identifier, 'title': 'Title of ' + identifier,
        'description': None, 'url': 'https://linear.app/x/issue/' + identifier,
        'team': {'id': 't-1', 'key': 'KK', 'name': 'Team'}, 'state': state('unstarted'),
        'assignee': None, 'project': None, 'projectMilestone': None, 'labels': {'nodes': []},
        'dueDate': None, 'estimate': None, 'sortOrder': 0.0, 'prioritySortOrder': 0.0,
        'createdAt': '2026-09-01T00:00:00Z', 'updatedAt': '2026-10-19T00:00:00Z',
        'startedAt': None, 'completedAt': None, 'canceledAt': None, 'parent': None,
        'attachments': {'nodes': []},
    }
    for k, v in kw.items():
        i[k] = v
    return i


def project(slug, **kw):
    p = {
        'id': 'p-' + slug, 'slugId': slug, 'name': 'Project ' + slug, 'url': 'https://linear.app/x/project/' + slug,
        'status': {'id': 'ps-started', 'name': 'In Progress', 'type': 'started'}, 'lead': None,
        'startDate': None, 'targetDate': None, 'health': None,
        'projectMilestones': {'nodes': []}, 'initiatives': {'nodes': []},
        'lastUpdate': update(slug, '2026-10-19T00:00:00Z'),
        'issues': {'nodes': [], 'pageInfo': {'hasNextPage': False, 'endCursor': None}},
        'updatedAt': '2026-10-19T00:00:00Z',
    }
    for k, v in kw.items():
        p[k] = v
    return p


def status(kind):
    return {'id': 'ps-' + kind, 'name': 'Status ' + kind, 'type': kind}


def update(slug, created):
    return {'id': 'su-' + slug, 'url': 'https://linear.app/x/project/%s/activity' % slug, 'body': 'Update.',
            'health': 'onTrack', 'createdAt': created, 'updatedAt': created, 'user': user(True), 'project': project_ref(slug)}


def issue_states(kinds):
    return {'nodes': [{'state': state(k)} for k in kinds], 'pageInfo': {'hasNextPage': False, 'endCursor': None}}


def snapshot(issues, projects, templates=None, workspace='ken109'):
    return {'workspace': workspace, 'issues': issues, 'projects': projects, 'templates': templates or []}


def label(name, group):
    return {'id': 'l-' + name, 'name': name, 'color': '#000000', 'isGroup': False,
            'parent': {'id': 'lg-' + group, 'name': group, 'groupType': 'singleSelect'}}


def source(url, metadata=None):
    return {'nodes': [{'id': 'a-1', 'title': 'Source', 'subtitle': None, 'url': url, 'sourceType': None, 'metadata': metadata or {}, 'createdAt': '2026-09-01T00:00:00Z'}]}


def heading(text):
    return {'type': 'heading', 'attrs': {'level': 2}, 'content': [{'type': 'text', 'text': text}]}


def template(name, sections):
    data = json.dumps({'title': '', 'descriptionData': {'type': 'doc', 'content': [heading(s) for s in sections]}}, separators=(',', ':'))
    return {'id': 'tpl-' + name, 'name': name, 'description': None, 'type': 'issue', 'team': None,
            'templateData': data, 'updatedAt': '2026-10-06T00:00:00Z'}


def rules_snapshot():
    me, other = user(True), user(False)
    projects = [
        project('done', status=status('completed'), lead=me, issues=issue_states(['completed', 'started'])),
        project('early', status=status('planned'), lead=other, issues=issue_states(['unstarted', 'started'])),
        project('late', lead=me, targetDate='2026-10-10',
                projectMilestones={'nodes': [milestone('late', 'M1', '2026-10-01', 'overdue')]}),
        project('nolead'),
        project('outdated', lead=me, lastUpdate=update('outdated', '2026-09-01T00:00:00Z')),
        project('ms', lead=me, projectMilestones={'nodes': [milestone('ms', 'Alpha', '2026-12-01', 'next')]}),
        project('ok', lead=me, issues=issue_states(['started', 'completed'])),
    ]
    issues = [
        issue('KK-1', state=state('started'), assignee=me, updatedAt='2026-10-01T00:00:00Z', startedAt='2026-09-20T00:00:00Z'),
        issue('KK-2', state=state('started'), assignee=other, updatedAt='2026-10-05T00:00:00Z', startedAt='2026-09-20T00:00:00Z'),
        issue('KK-3', assignee=me, dueDate='2026-10-15'),
        issue('KK-4', state=state('started'), assignee=me, project=project_ref('ms'), startedAt='2026-10-10T00:00:00Z'),
        issue('KK-5', state=state('completed'), assignee=me, dueDate='2026-10-01', completedAt='2026-10-02T00:00:00Z'),
        issue('KK-6', state=state('started'), assignee=me, project=project_ref('ok'), startedAt='2026-10-18T00:00:00Z'),
    ]
    return snapshot(issues, projects)


def clean_snapshot():
    me = user(True)
    return snapshot(
        [issue('KK-6', state=state('started'), assignee=me, project=project_ref('ok'), startedAt='2026-10-18T00:00:00Z')],
        [project('ok', lead=me, issues=issue_states(['started', 'completed']))],
    )


def validators_snapshot():
    me = user(True)
    body_ok = '## Goal\nShip it.\n\n## Done when\nIt is shipped.\n'
    issues = [
        issue('KK-10', assignee=me, description=body_ok),
        issue('KK-11', assignee=me, description=body_ok, attachments=source('https://example.com/a'),
              labels={'nodes': [label('Bug', 'Type'), label('Feature', 'Type')]}),
        issue('KK-12', assignee=me, description='## Goal\n\n## Notes\nSomething.\n', attachments=source('https://example.com/b')),
        issue('KK-13', assignee=me, description=body_ok, attachments=source('https://example.com/c'),
              labels={'nodes': [label('Bug', 'Type')]}),
        issue('KK-14', assignee=user(False), description=None, attachments=source('https://example.com/d')),
    ]
    return snapshot(issues, [], [template('Task', ['Goal', 'Done when'])])


def source_kinds_snapshot():
    me = user(True)
    return snapshot([
        issue('KK-20', assignee=me, attachments=source('https://example.com/a', {'kind': 'slack', 'ticket': 7})),
        issue('KK-21', assignee=me, attachments=source('https://example.com/b', {'kind': 'email'})),
        issue('KK-22', assignee=me, attachments=source('https://example.com/c')),
        issue('KK-23', assignee=me),
    ], [])


def real_snapshot():
    issue_r = json.load(open(FIX + 'issue.json'))['data']['issue']
    projects_r = json.load(open(FIX + 'projects.json'))['data']['projects']['nodes']
    return snapshot([issue_r], projects_r, workspace='example')


def phase1():
    rules = rules_snapshot()
    write('audit', 'rules-default', 'Every consistency and staleness rule fires once, at the default thresholds.',
          {'snapshot': rules, 'config': None, 'options': None, 'now': NOW})
    write('audit', 'rules-lenient-thresholds', 'The same snapshot with thresholds so long that the staleness rules stay quiet.',
          {'snapshot': rules, 'config': {'staleDays': 30, 'statusUpdateDays': 60, 'validators': []}, 'options': None, 'now': NOW})
    write('audit', 'rules-an-empty-config-is-the-default', 'An empty config object means the defaults.',
          {'snapshot': rules, 'config': {}, 'options': None, 'now': NOW})
    write('audit', 'scoped-to-issues-since', 'Narrowed to named issues (matched without regard to case), one unknown, with a since time.',
          {'snapshot': rules, 'config': None, 'options': {'issues': ['kk-1', 'KK-6', 'KK-999'], 'since': '2026-10-10T00:00:00Z'}, 'now': NOW})
    write('audit', 'scoped-without-since', 'Narrowed to the issues and their projects, no since time.',
          {'snapshot': rules, 'config': None, 'options': {'issues': ['KK-4']}, 'now': NOW})
    write('audit', 'validators-on-existing-issues', 'source-attachment, label-groups-exclusive and template-sections applied to existing issues.',
          {'snapshot': validators_snapshot(), 'config': {'staleDays': 7, 'statusUpdateDays': 14,
                                                         'validators': ['source-attachment', 'label-groups-exclusive', 'template-sections']},
           'options': None, 'now': NOW})
    write('audit', 'validators-source-kinds', 'source-attachment with sourceKinds: the source attachment needs a metadata.kind from the list.',
          {'snapshot': source_kinds_snapshot(), 'config': {'staleDays': 7, 'statusUpdateDays': 14,
                                                           'validators': ['source-attachment'],
                                                           'sourceKinds': ['slack', 'life-decision']},
           'options': None, 'now': NOW})
    write('audit', 'healthy-workspace', 'Nothing is wrong: no findings.',
          {'snapshot': clean_snapshot(), 'config': None, 'options': None, 'now': NOW})
    write('audit', 'empty-snapshot', 'No issues and no projects.',
          {'snapshot': snapshot([], []), 'config': None, 'options': None, 'now': NOW})
    write('audit', 'anonymized-real-responses', "The issue and projects of the anonymized sandbox responses, with thresholds of one day, a fortnight later.",
          {'snapshot': real_snapshot(), 'config': {'staleDays': 1, 'statusUpdateDays': 1, 'validators': []}, 'options': None, 'now': NOW})
    write('audit', 'error-since-without-issues', 'since has no issues to be about: a usage error.',
          {'snapshot': rules, 'config': None, 'options': {'since': '2026-10-10T00:00:00Z'}, 'now': NOW})
    write('audit', 'error-snapshot-is-not-a-snapshot', 'A snapshot without its fields is a usage error, not a panic.',
          {'snapshot': {}, 'config': None, 'options': None, 'now': NOW})
    write('audit', 'error-unknown-config-key', 'An unknown key in the config is refused rather than ignored.',
          {'snapshot': clean_snapshot(), 'config': {'staleDay': 3}, 'options': None, 'now': NOW})

    # ---- refresh
    base = {'schemaVersion': 3}
    cases = [
        ('never-fetched', 'No cache at all.', None, {'kind': 'read'}),
        ('fresh-read', 'Fetched a minute ago: leave it.', {**base, 'fetchedAt': '2026-10-20T11:59:00Z'}, {'kind': 'read'}),
        ('read-at-the-ttl-edge', 'Exactly at the TTL is still within it.', {**base, 'fetchedAt': '2026-10-20T11:55:00Z'}, {'kind': 'read'}),
        ('read-past-the-ttl', 'One second past the TTL: refresh.', {**base, 'fetchedAt': '2026-10-20T11:54:59Z'}, {'kind': 'read'}),
        ('custom-ttl', 'A short TTL set by the caller.', {**base, 'fetchedAt': '2026-10-20T11:59:00Z', 'ttlSecs': 30}, {'kind': 'read'}),
        ('cron-half-way', 'A cron tick finds the snapshot half way to stale.', {**base, 'fetchedAt': '2026-10-20T11:57:30Z'}, {'kind': 'cron'}),
        ('cron-too-early', 'A cron tick finds it fresh.', {**base, 'fetchedAt': '2026-10-20T11:58:00Z'}, {'kind': 'cron'}),
        ('relevant-webhook', 'An Issue webhook refreshes even a fresh cache.', {**base, 'fetchedAt': '2026-10-20T11:59:50Z'},
         {'kind': 'webhook', 'resourceType': 'Issue', 'action': 'update'}),
        ('irrelevant-webhook', 'A Comment webhook changes nothing the snapshot holds.', {**base, 'fetchedAt': '2026-10-20T11:59:50Z'},
         {'kind': 'webhook', 'resourceType': 'Comment', 'action': 'create'}),
        ('manual-overrides-backoff', 'A person asks right after a failure.',
         {**base, 'fetchedAt': '2026-10-20T10:00:00Z', 'lastFailureAt': '2026-10-20T11:59:55Z'}, {'kind': 'manual'}),
        ('backing-off-after-a-failure', 'A failure a moment ago: leave Linear alone, and the old snapshot is expired.',
         {**base, 'fetchedAt': '2026-10-20T10:00:00Z', 'lastFailureAt': '2026-10-20T11:59:30Z'}, {'kind': 'read'}),
        ('another-schema-version', 'A cache written with another schema version counts as missing.',
         {'schemaVersion': 99, 'fetchedAt': '2026-10-20T11:59:50Z'}, {'kind': 'read'}),
        ('clock-skew', 'A snapshot dated in the future counts as just fetched.', {**base, 'fetchedAt': '2026-10-20T12:00:30Z'}, {'kind': 'read'}),
        ('error-unknown-event', 'An event kind that does not exist is a usage error.', None, {'kind': 'nope'}),
        ('error-meta-has-an-unknown-key', 'An unknown key in the meta is refused.', {**base, 'fetched': '2026-10-20T12:00:00Z'}, {'kind': 'read'}),
    ]
    for name, desc, meta, event in cases:
        write('refresh', name, desc, {'meta': meta, 'event': event, 'now': NOW})

    # ---- webhook
    secret = 'lin_wh_goldensecret'

    def sign(body, key=secret):
        return hmac.new(key.encode(), body.encode(), hashlib.sha256).hexdigest()

    ts = 1792497600000  # NOW
    body = json.dumps({'action': 'update', 'type': 'Issue', 'organizationId': 'org-1', 'webhookTimestamp': ts}, separators=(',', ':'))
    jp = json.dumps({'action': 'create', 'type': 'Issue', 'data': {'title': '日本語の題名 — emoji 😀'}, 'webhookTimestamp': ts},
                    separators=(',', ':'), ensure_ascii=False)
    no_ts = json.dumps({'action': 'create', 'type': 'Issue'}, separators=(',', ':'))
    wnow = NOW
    write('webhook', 'valid', 'A correctly signed, current delivery.', {'body': body, 'signature': sign(body), 'secret': secret, 'now': wnow})
    write('webhook', 'valid-upper-case-signature', 'The digest in upper case is the same digest.', {'body': body, 'signature': sign(body).upper(), 'secret': secret, 'now': wnow})
    write('webhook', 'valid-multibyte-body', 'A body with Japanese text and an emoji: the bytes signed are UTF-8, on both sides.',
          {'body': jp, 'signature': sign(jp), 'secret': secret, 'now': wnow})
    write('webhook', 'valid-at-the-edge-of-the-window', 'Sixty seconds late is still inside the window.',
          {'body': body, 'signature': sign(body), 'secret': secret, 'now': '2026-10-20T12:01:00Z'})
    write('webhook', 'stale', 'One millisecond past the window.', {'body': body, 'signature': sign(body), 'secret': secret, 'now': '2026-10-20T12:01:00.001Z'})
    write('webhook', 'from-the-future', 'Too early is as suspect as too late.', {'body': body, 'signature': sign(body), 'secret': secret, 'now': '2026-10-20T11:58:59Z'})
    write('webhook', 'wrong-secret', 'Signed with another secret.', {'body': body, 'signature': sign(body, 'other'), 'secret': secret, 'now': wnow})
    write('webhook', 'tampered-body', 'The body changed after signing.', {'body': body.replace('update', 'remove'), 'signature': sign(body), 'secret': secret, 'now': wnow})
    write('webhook', 'malformed-signature', 'Not 64 hex digits.', {'body': body, 'signature': 'sha256=' + sign(body), 'secret': secret, 'now': wnow})
    write('webhook', 'signed-but-no-timestamp', 'A valid signature over a body without webhookTimestamp.', {'body': no_ts, 'signature': sign(no_ts), 'secret': secret, 'now': wnow})
    write('webhook', 'signed-but-not-json', 'A valid signature over text that is not a payload.', {'body': 'hello', 'signature': sign('hello'), 'secret': secret, 'now': wnow})
    write('webhook', 'error-empty-secret', 'An empty secret is a configuration error, not a rejected delivery.', {'body': body, 'signature': sign(body), 'secret': '', 'now': wnow})

    # ---- parse_response
    fixtures = [
        ('whoami', 'whoami'), ('issue', 'issue'), ('assigned_started_issues', 'assigned_issues'), ('projects', 'projects'),
        ('issue_comments', 'issue_comments'), ('templates', 'templates'), ('initiatives', 'initiatives'), ('issue_view', 'issue_view'),
        ('project_view', 'project_view'), ('milestones_of_project', 'milestones'), ('milestone_view', 'milestone_view'),
        ('initiative_view', 'initiative_view'), ('labels', 'labels'), ('teams', 'teams'), ('users', 'users'),
    ]
    for op, fx in fixtures:
        write('parse', op, 'The anonymized response `%s`, read as `%s`.' % (fx, op),
              {'operation': op, 'fixture': fx, 'status': 200, 'now': NOW})
    write('parse', 'templates-with-sections', 'Templates with real heading nodes and a project template.',
          {'operation': 'templates', 'fixture': 'templates_sections', 'status': 200, 'now': NOW})
    write('parse', 'error-unauthenticated', 'A rejected key.', {'operation': 'whoami', 'fixture': 'error_unauthenticated', 'status': 401, 'now': NOW})
    write('parse', 'error-query-too-complex', "Linear's own error code is passed on.", {'operation': 'projects', 'fixture': 'error_too_complex', 'status': 400, 'now': NOW})
    write('parse', 'error-rate-limited-until', 'The wait comes from the reset timestamp (30 s after now).',
          {'operation': 'whoami', 'fixture': None, 'body': '{}', 'status': 429, 'rateLimitResetMs': 1792497630000, 'now': NOW})
    write('parse', 'error-rate-limited-retry-after', 'The wait comes from Retry-After.',
          {'operation': 'whoami', 'fixture': None, 'body': '{}', 'status': 429, 'retryAfterSecs': 7, 'now': NOW})
    write('parse', 'error-html-body', 'A body that is not a GraphQL response.',
          {'operation': 'whoami', 'fixture': None, 'body': '<html>bad gateway</html>', 'status': 200, 'now': NOW})
    write('parse', 'error-http-500', 'A server error without a GraphQL body keeps the status and a truncated body.',
          {'operation': 'whoami', 'fixture': None, 'body': 'x' * 400, 'status': 500, 'now': NOW})
    write('parse', 'error-empty-response', 'Neither data nor errors.',
          {'operation': 'whoami', 'fixture': None, 'body': '{}', 'status': 200, 'now': NOW})
    write('parse', 'error-wrong-shape', 'A response for another query.',
          {'operation': 'issue', 'fixture': 'whoami', 'status': 200, 'now': NOW})
    write('parse', 'error-unknown-operation', 'An operation that is not in the table.',
          {'operation': 'nope', 'fixture': None, 'body': '{}', 'status': 200, 'now': NOW})

    # ---- build_request
    for op, params, desc in [
        ('whoami', None, 'No parameters.'),
        ('whoami', {}, 'An empty object for no parameters.'),
        ('issue', {'id': 'KK-12'}, 'An issue by identifier.'),
        ('issue_view', {'id': 'KK-12'}, 'An issue with its detail.'),
        ('issue_comments', {'id': 'KK-12'}, "An issue's comments."),
        ('assigned_started_issues', None, 'The first page at the default size.'),
        ('projects', None, 'The first page at the size that stays under the complexity limit.'),
        ('projects', {'first': 3, 'after': 'cursor-1'}, 'A later page.'),
        ('project_view', {'id': 'my-project-1a2b3c'}, 'A project by slug.'),
        ('milestones_of_project', {'id': 'p-1'}, "A project's milestones."),
        ('milestone_view', {'id': 'm-1'}, 'A milestone.'),
        ('initiatives', {'first': 5}, 'Initiatives.'),
        ('initiative_view', {'id': 'i-1'}, 'An initiative.'),
        ('labels', None, 'Labels.'),
        ('teams', None, 'Teams.'),
        ('users', {'includeDisabled': True}, 'Users including disabled ones.'),
        ('users', None, 'Users, with includeDisabled left to Linear.'),
        ('templates', None, 'Templates.'),
    ]:
        name = op + ('' if params is None else '-' + '-'.join(sorted(params))) if params != {} else op + '-empty-object'
        write('build', name, desc, {'operation': op, 'params': params})
    write('build', 'error-missing-id', 'An id is required.', {'operation': 'issue', 'params': {}})
    write('build', 'error-unknown-parameter', 'An unknown parameter is refused.', {'operation': 'whoami', 'params': {'id': 'x'}})
    write('build', 'error-unknown-operation', 'An operation that is not in the table.', {'operation': 'nope', 'params': None})
    write('build', 'unicode-id', 'An id with non-ASCII text survives into the variables.', {'operation': 'issue', 'params': {'id': 'KK-1 日本語'}})


def phase2():
    """diff cases built from the audit reports phase 1 produced."""
    def report(name):
        return json.load(open(OUT + 'audit/%s.json' % name))['expected']['data']

    full = report('rules-default')
    findings = full['findings']
    assert len(findings) >= 4, len(findings)
    write('diff', 'no-previous-report', 'There was no earlier audit: everything is new.', {'previous': None, 'current': full})
    write('diff', 'same-report', 'Nothing is new.', {'previous': full, 'current': full})
    older = {'findings': findings[:-2], 'unresolvedIssues': []}
    write('diff', 'two-new-findings', 'The last two findings are new, in the order of the current report.', {'previous': older, 'current': full})
    reworded = copy.deepcopy(full)
    for f in reworded['findings']:
        f['message'] = f['message'] + ' (reworded)'
    write('diff', 'a-changed-message-is-not-new', 'The message carries day counts that change daily: the same finding is not new.', {'previous': full, 'current': reworded})
    gone = {'findings': findings[1:], 'unresolvedIssues': []}
    write('diff', 'a-resolved-finding-is-not-reported', 'A finding that went away is not "new".', {'previous': full, 'current': gone})
    write('diff', 'both-empty', 'Nothing before, nothing now.', {'previous': {'findings': [], 'unresolvedIssues': []}, 'current': {'findings': [], 'unresolvedIssues': []}})
    write('diff', 'error-current-is-not-a-report', 'The current report must be a report.', {'previous': None, 'current': {'findings': 'no'}})


if __name__ == '__main__':
    {'phase1': phase1, 'phase2': phase2}[sys.argv[1]]()
