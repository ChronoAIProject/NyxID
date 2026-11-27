/** Shared vector props -> editable Rive nodes/timelines. No cloud authoring. */
import { readFile, writeFile } from "node:fs/promises";
const art = JSON.parse(await readFile(new URL("prop-art.json", import.meta.url)));
const sceneUrl = new URL("scene.rml", import.meta.url);
const source = await readFile(sceneUrl, "utf8");
const ids = new Map();
let nextId = 1000;
function collect(nodes) {
  for (const node of nodes) {
    ids.set(node.name, { id: `0:${nextId++}`, node });
    collect(node.children);
  }
}
collect(art.roots);
const a = (attributes) => Object.entries(attributes).map(([key, value]) => ` ${key}="${value}"`).join("");
function shape(s) {
  const paint = `${s.fill !== "none" ? `<Fill><SolidColor colorValue="FF${s.fill.slice(1)}"/></Fill>` : ""}${s.stroke ? `<Stroke thickness="${s.strokeWidth}" cap="round" join="round"><SolidColor colorValue="FF${s.stroke.slice(1)}"/></Stroke>` : ""}`;
  if (s.type === "rect") return `<Shape x="${s.x+s.width/2}" y="${s.y+s.height/2}"><Rectangle width="${s.width}" height="${s.height}" cornerRadiusTL="${s.rx}"/>${paint}</Shape>`;
  if (s.type === "circle") return `<Shape x="${s.cx}" y="${s.cy}"><Ellipse width="${s.r*2}" height="${s.r*2}"/>${paint}</Shape>`;
  return `<Shape><PointsPath isClosed="${s.type === "polygon"}">${s.points.map(([x,y]) => `<StraightVertex x="${x}" y="${y}"/>`).join("")}</PointsPath>${paint}</Shape>`;
}
function node(n) {
  return `<Node${a({name:n.name,id:ids.get(n.name).id,x:n.x,y:n.y,rotation:n.rotation,scaleX:n.scaleX,scaleY:n.scaleY})}>${[...n.children].reverse().map(node).join("")}${[...n.shapes].reverse().map(shape).join("")}</Node>`;
}
const frames = (values, mode="cubic") => values.map(([frame,value,easing=mode]) => {
  const curves={press:[0.45,0,0.85,1],release:[0.12,0.7,0.3,1],cubic:[0.35,0,0.25,1]};
  const curve=curves[easing];
  return `<KeyFrameDouble frame="${frame}" value="${value}" interpolationType="${curve ? 'cubic' : easing}">${curve ? `<CubicEaseInterpolator x1="${curve[0]}" y1="${curve[1]}" x2="${curve[2]}" y2="${curve[3]}"/>` : ''}</KeyFrameDouble>`;
}).join("");
const key = (property, values, mode) => `<KeyedProperty property="${property}">${frames(values,mode)}</KeyedProperty>`;
const repeat = (duration, length, sequence) => {
  const result=[];
  for(let start=0; start<duration;start+=length) for(const [t,v] of sequence) if(start+t<duration) result.push([start+t,v]);
  result.push([duration,sequence[0][1]]);
  return result.filter(([t],i)=>!i || result[i-1][0]!==t);
};
function animation(emote, duration) {
  const visible = {1:"Thought",2:"Keyboard",3:"Tool",4:"Clock",5:"Request",6:"Retry",7:"Stop",8:"Warning",11:"Keyboard"}[emote];
  return [...ids].map(([name,{id,node:n}]) => {
    const root = art.roots.includes(n);
    const properties = {x:[[0,n.x]], y:[[0,n.y]], rotation:[[0,n.rotation]], scaleX:[[0,root ? +(name===visible) : 1]], scaleY:[[0,root ? +(name===visible) : 1]]};
    if(emote===2) {
      const left=[18,39,71,132,154,183,254,281,354,383,412];
      const right=[27,55,87,141,168,236,266,301,366,399];
      const tapping=['TapLeft','KeyLeft','TapRight','KeyRight'].includes(name);
      if(tapping) {
        const beats=name.endsWith('Left')?left:right;
        const hand=name.startsWith('Tap');
        properties.y=[[0,n.y,'hold']];
        for(const beat of beats)properties.y.push([beat-4,n.y,'press'],[beat,n.y+(hand?5:1.7),'release'],[beat+(hand?8:4),n.y,'hold']);
        properties.y.push([duration,n.y]);
        if(hand) {
          properties.scaleX=[[0,1,'hold']];properties.scaleY=[[0,1,'hold']];
          for(const beat of beats) {
            properties.scaleX.push([beat-4,1,'press'],[beat,1.06,'release'],[beat+8,1,'hold']);
            properties.scaleY.push([beat-4,1,'press'],[beat,0.92,'release'],[beat+8,1,'hold']);
          }
          properties.scaleX.push([duration,1]);properties.scaleY.push([duration,1]);
        }
      }
    }
    if(emote===3) {
      const strokes=[{length:116,start:14,end:46,angle:Math.PI/6},{length:144,start:21,end:61,angle:Math.PI/6},{length:126,start:12,end:55,angle:Math.PI/3},{length:154,start:27,end:70,angle:Math.PI/6},{length:120,start:15,end:49,angle:Math.PI/6}];
      if(name==='Wrench'||name==='Bolt') {
        properties.rotation=[];
        if(name==='Wrench') { properties.x=[];properties.y=[]; }
        let at=0,turned=0;
        for(const stroke of strokes) {
          const {length,start,end,angle}=stroke;
          if(name==='Bolt') properties.rotation.push([at,turned,'hold'],[at+start,turned,'cubic'],[at+end,turned+angle,'hold']);
          else {
            properties.rotation.push([at,-2.3,'hold'],[at+start,-2.3,'cubic'],[at+end,-2.3+angle,'hold'],[at+end+18,-2.3+angle,'cubic'],[at+length-22,-2.3,'hold']);
            for(const [key,offset] of [['x',5],['y',-7]]) properties[key].push([at,0,'hold'],[at+end+4,0,'cubic'],[at+end+15,offset,'hold'],[at+length-24,offset,'cubic'],[at+length-10,0,'hold']);
          }
          turned+=angle;at+=length;
        }
        properties.rotation.push([duration,name==='Bolt'?turned:-2.3]);
        if(name==='Wrench'){properties.x.push([duration,0]);properties.y.push([duration,0]);}
      }
    }
    if(emote===4 && name==='Minute') {
      properties.rotation=[[0,0,'hold']];
      for(let tick=1;tick<=12;tick++) {
        const t=tick*50;
        properties.rotation.push([t-8,(tick-1)*Math.PI/6,'cubic'],[t-3,tick*Math.PI/6+0.016,'release'],[t,tick*Math.PI/6,'hold']);
      }
    }
    // The clock indicates waiting, not elapsed wall-clock time or a countdown.
    if(emote===1 && name.startsWith('Thought') && name!=='Thought') {
      const delay={ThoughtA:0,ThoughtB:16,ThoughtC:32}[name];
      properties.y=repeat(duration,180,[[0,n.y,'hold'],[delay+20,n.y,'cubic'],[delay+35,n.y-0.8,'cubic'],[delay+58,n.y,'hold'],[180,n.y]]);
      properties.scaleX=repeat(duration,180,[[0,0.82,'hold'],[delay+20,0.82,'cubic'],[delay+35,1.08,'cubic'],[delay+58,0.82,'hold'],[180,0.82]]);
      properties.scaleY=properties.scaleX;
    }
    if(emote===6 && name==='RetryArrow')properties.rotation=[[0,-0.4],[72,Math.PI*2-0.4],[duration,Math.PI*2-0.4]];
    if(emote===7 && name==='Stop') {properties.scaleX=[[0,0.82],[18,1]];properties.scaleY=properties.scaleX;}
    if(emote===8 && name==='Warning') {properties.rotation=[[0,-0.1],[10,0.08],[22,0]];}
    if(emote===11 && name==='Keyboard') {
      properties.y=[[0,n.y],[duration,n.y+18]];
      properties.scaleX=[[0,1],[duration,0]]; properties.scaleY=properties.scaleX;
    }
    const content=Object.entries(properties).map(([property,values])=>key(property,values,emote===4 && name==='Minute' ? 'linear':undefined)).join('');
    return `<KeyedObject objectId="${id}">${content}</KeyedObject>`;
  }).join('\n');
}
let updated=source.replace(/    <!-- PROPS ART START -->[\s\S]*?    <!-- PROPS ART END -->\n/g,'').replace(/\s*<!-- PROP KEYS START -->[\s\S]*?<!-- PROP KEYS END -->/g,'');
updated=updated.replace('    <StateMachine name="Emotes"',`    <!-- PROPS ART START -->\n${[...art.roots].reverse().map(node).join('\n')}\n    <!-- PROPS ART END -->\n    <StateMachine name="Emotes"`);
let index=0;
updated=updated.replace(/(<LinearAnimation\b[^>]*duration="(\d+)"[^>]*>)([\s\S]*?)(<\/LinearAnimation>)/g,(_,open,duration,body,close)=>`${open}${body.trimEnd()}\n<!-- PROP KEYS START -->\n${animation(index++,Number(duration))}\n<!-- PROP KEYS END -->\n${close}`);
if(index!==12)throw Error('Expected all twelve emote timelines');
if(process.argv.includes('--check')) {if(updated!==source)throw Error('Run generate-props.mjs to refresh Rive prop artwork/timelines');}
else await writeFile(sceneUrl,updated);
console.log(`${ids.size} Rive prop nodes; twelve complete timelines ${process.argv.includes('--check')?'checked':'generated'}.`);
