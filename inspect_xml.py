import csv, html, xml.etree.ElementTree as ET
from collections import Counter, defaultdict
from pathlib import Path

BASE = Path(__file__).resolve().parent
XML_FILE = BASE / 'data' / '2647319.xml'
OUTPUT = BASE / 'output'; MAPPINGS = BASE / 'mappings'
OUTPUT.mkdir(exist_ok=True)

def load_csv(path, key):
    with path.open(encoding='utf-8-sig', newline='') as f:
        return {r[key]: r for r in csv.DictReader(f)}

def esc(v): return html.escape('' if v is None else str(v))
def write_csv(path, fields, rows):
    with path.open('w', newline='', encoding='utf-8-sig') as f:
        w=csv.DictWriter(f, fieldnames=fields); w.writeheader(); w.writerows(rows)

def table(headers, rows):
    s=['<table><thead><tr>']+[f'<th>{esc(x)}</th>' for x in headers]+['</tr></thead><tbody>']
    for row in rows:
        s.append('<tr>'+''.join(f'<td>{esc(x)}</td>' for x in row)+'</tr>')
    s.append('</tbody></table>'); return ''.join(s)

def main():
    root=ET.parse(XML_FILE).getroot(); games=root.findall('Game'); events=root.findall('.//Event'); qs=root.findall('.//Q')
    em=load_csv(MAPPINGS/'event_types.csv','type_id'); qm=load_csv(MAPPINGS/'qualifiers.csv','qualifier_id')
    et=Counter(e.get('type_id') for e in events if e.get('type_id')); qct=Counter(q.get('qualifier_id') for q in qs if q.get('qualifier_id'))
    players={e.get('player_id'):e.get('player_name','') for e in events if e.get('player_id')}
    teams={}
    for g in games:
        teams[g.get('home_team_id')]=g.get('home_team_name',''); teams[g.get('away_team_id')]=g.get('away_team_name','')

    event_rows=[]; qualifier_rows=[]
    for e in events:
        tid=e.get('type_id',''); m=em.get(tid,{})
        event_rows.append({
            'event_id':e.get('id',''),'type_id':tid,'event_type':m.get('name','Unknown / not mapped'),
            'event_type_description':m.get('description',''),'mapping_status':m.get('source_status','unknown'),
            'period_id':e.get('period_id',''),'minute':e.get('min',''),'second':e.get('sec',''),
            'team_id':e.get('team_id',''),'player_id':e.get('player_id',''),'player_name':e.get('player_name',''),
            'x':e.get('x',''),'y':e.get('y',''),'outcome':e.get('outcome',''),
            'timestamp':e.get('timestamp',''),'last_modified':e.get('last_modified',''),
            'qualifier_count':len(e.findall('Q'))})
        for q in e.findall('Q'):
            qid=q.get('qualifier_id',''); m2=qm.get(qid,{})
            qualifier_rows.append({'event_id':e.get('id',''),'event_type_id':tid,'event_type':m.get('name','Unknown / not mapped'),
                'player_name':e.get('player_name',''),'qualifier_id':qid,'qualifier_name':m2.get('name','Unknown / not mapped'),
                'qualifier_description':m2.get('description',''),'mapping_status':m2.get('source_status','unknown'),'value':q.get('value','')})

    write_csv(OUTPUT/'events_with_meanings.csv', list(event_rows[0].keys()), event_rows)
    write_csv(OUTPUT/'qualifiers_with_meanings.csv', list(qualifier_rows[0].keys()), qualifier_rows)

    # Extend summary files with meanings.
    type_summary=[]
    for tid,c in sorted(et.items(), key=lambda x:int(x[0])):
        m=em.get(tid,{})
        type_summary.append({'type_id':tid,'name':m.get('name','Unknown / not mapped'),'description':m.get('description',''),'occurrences':c,'source_status':m.get('source_status','unknown')})
    write_csv(OUTPUT/'event_type_summary_with_meanings.csv',['type_id','name','description','occurrences','source_status'],type_summary)

    qvalues=defaultdict(set)
    for q in qs:
        if q.get('value') not in (None,''): qvalues[q.get('qualifier_id')].add(q.get('value'))
    qsum=[]
    for qid,c in sorted(qct.items(), key=lambda x:int(x[0])):
        m=qm.get(qid,{})
        qsum.append({'qualifier_id':qid,'name':m.get('name','Unknown / not mapped'),'description':m.get('description',''),
                     'occurrences':c,'example_values':' | '.join(sorted(qvalues[qid])[:10]),'source_status':m.get('source_status','unknown')})
    write_csv(OUTPUT/'qualifier_summary_with_meanings.csv',['qualifier_id','name','description','occurrences','example_values','source_status'],qsum)

    unknown_e=[x for x in type_summary if x['source_status']=='unknown']; unknown_q=[x for x in qsum if x['source_status']=='unknown']
    g=games[0] if games else {}
    sample=[]
    for e in events[:50]:
        m=em.get(e.get('type_id'),{}); sample.append([e.get('id',''),e.get('type_id',''),m.get('name','Unknown / not mapped'),e.get('period_id',''),f"{e.get('min','')}:{e.get('sec','')}",e.get('player_name',''),e.get('x',''),e.get('y',''),e.get('outcome',''),e.get('timestamp',''),len(e.findall('Q'))])

    evdict=[[r['type_id'],r['name'],r['description'],r['occurrences'],r['source_status']] for r in type_summary]
    qdict=[[r['qualifier_id'],r['name'],r['description'],r['occurrences'],r['example_values'],r['source_status']] for r in qsum]
    prow=[[pid,name] for pid,name in sorted(players.items(), key=lambda x:x[1].lower())]
    trow=[[tid,name] for tid,name in teams.items()]

    css='''body{font-family:Arial,sans-serif;margin:0;background:#f4f6f8;color:#202124}header{background:#1f2937;color:white;padding:28px 40px}nav{background:white;padding:12px 30px;position:sticky;top:0;border-bottom:1px solid #ddd}nav a{margin-right:15px;color:#2563eb;text-decoration:none;font-size:13px}main{max-width:1500px;margin:25px auto;padding:0 20px}section{background:white;margin-bottom:22px;padding:22px;border-radius:9px;box-shadow:0 1px 4px #0001}table{width:100%;border-collapse:collapse;font-size:13px;margin-top:12px}th,td{border:1px solid #ddd;padding:7px;vertical-align:top;text-align:left}th{background:#f3f4f6}tr:nth-child(even){background:#fafafa}.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:12px}.card{padding:16px;background:#f8fafc;border:1px solid #ddd;border-radius:8px}.n{font-size:26px;font-weight:bold}.note{padding:13px;background:#fff7ed;border-left:4px solid #f97316;margin:12px 0}.unknown{background:#fff1f2}.small{color:#666;font-size:12px}'''
    report=f'''<!doctype html><html><head><meta charset="utf-8"><title>Football XML Data Report</title><style>{css}</style></head><body>
<header><h1>Football XML Data Report</h1><div>Source: {esc(XML_FILE.name)}</div><div>Opta/F24-style event and qualifier interpretation layer</div></header>
<nav><a href="#overview">Overview</a><a href="#events">Events</a><a href="#eventdict">Event Dictionary</a><a href="#qualifiers">Qualifiers</a><a href="#qualdict">Qualifier Dictionary</a><a href="#players">Players</a><a href="#samples">Samples</a></nav><main>
<section id="overview"><h2>1. Match & Overview</h2><div class="cards"><div class="card"><div class="n">{len(events)}</div>Events</div><div class="card"><div class="n">{len(et)}</div>Event IDs</div><div class="card"><div class="n">{len(qct)}</div>Qualifier IDs</div><div class="card"><div class="n">{len(players)}</div>Players</div><div class="card"><div class="n">{len(unknown_e)}</div>Unmapped Event IDs</div><div class="card"><div class="n">{len(unknown_q)}</div>Unmapped Qualifier IDs</div></div><h3>Match</h3>{table(['Field','Value'],[['Match ID',g.get('id','')],['Home',g.get('home_team_name','')],['Away',g.get('away_team_name','')],['Competition ID',g.get('competition_id','')],['Season',g.get('season','')],['Date',g.get('game_date','')]])}<div class="note"><b>Important:</b> meanings marked <b>reference</b> come from the public Opta F24 reference. IDs marked <b>unknown</b> occur in your XML but were not found in that reference and are not guessed.</div></section>
<section id="events"><h2>2. Event Types in This XML</h2>{table(['ID','Name','Occurrences'],[[r['type_id'],r['name'],r['occurrences']] for r in type_summary])}</section>
<section id="eventdict"><h2>3. Event Type Dictionary</h2>{table(['Type ID','Meaning','Description','Occurrences','Status'],evdict)}</section>
<section id="qualifiers"><h2>4. Qualifiers in This XML</h2>{table(['ID','Name','Occurrences','Example Values'],[[r['qualifier_id'],r['name'],r['occurrences'],r['example_values']] for r in qsum])}</section>
<section id="qualdict"><h2>5. Qualifier Dictionary</h2>{table(['Qualifier ID','Meaning','Description','Occurrences','Example Values','Status'],qdict)}</section>
<section id="players"><h2>6. Players</h2>{table(['Player ID','Player Name'],prow)}<h3>Teams</h3>{table(['Team ID','Team Name'],trow)}</section>
<section id="samples"><h2>7. First 50 Events with Meanings</h2>{table(['Event ID','Type ID','Event Type','Period','Time','Player','X','Y','Outcome','Timestamp','Qualifiers'],sample)}</section>
<section><h2>8. Generated Files</h2>{table(['File','Purpose'],[['events_with_meanings.csv','Every Event with type name/description and raw attributes.'],['qualifiers_with_meanings.csv','Every qualifier with name/description and raw value.'],['event_type_summary_with_meanings.csv','Event type counts plus meanings.'],['qualifier_summary_with_meanings.csv','Qualifier counts, meanings and example values.'],['mappings/event_types.csv','Mapping dictionary for event type IDs.'],['mappings/qualifiers.csv','Mapping dictionary for qualifier IDs.']])}</section>
</main></body></html>'''
    (OUTPUT/'xml_report.html').write_text(report,encoding='utf-8')
    print(f'XML: {len(events)} events, {len(qct)} qualifier IDs, {len(et)} event IDs')
    print(f'Mapped event IDs: {len(et)-len(unknown_e)}; unknown: {[x["type_id"] for x in unknown_e]}')
    print(f'Mapped qualifier IDs: {len(qct)-len(unknown_q)}; unknown: {[x["qualifier_id"] for x in unknown_q]}')
    print(f'Report: {OUTPUT/"xml_report.html"}')

if __name__=='__main__': main()
